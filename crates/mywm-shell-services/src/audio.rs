//! PipeWire's outputs, microphones and application streams with their volume and mute, and which output and
//! microphone are the default (`default.audio.sink` and `default.audio.source` of the `default` metadata), as the
//! Quickshell bar's audio panel lists them: microphones without WirePlumber's smart filters (the NixOS mic filter
//! puts itself in front of the default microphone and never becomes the default), streams only with an
//! application name.
use std::{cell::RefCell, rc::Rc, sync::mpsc::Sender};

use mywm_shell_protocol::{Audio, AudioNode, NodeKind};
use pipewire as pw;
use pw::{
    metadata::{Metadata, MetadataListener},
    node::{Node, NodeChangeMask, NodeListener},
    spa::{
        param::ParamType,
        pod::{Pod, Value, ValueArray, deserialize::PodDeserializer},
    },
    types::ObjectType,
};

use crate::{RETRY, Update};

const CHANNEL_VOLUMES: u32 = pw::spa::sys::SPA_PROP_channelVolumes;
const MUTE: u32 = pw::spa::sys::SPA_PROP_mute;

/// What the graph says, as the compositor gets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<AudioNode>,
    pub default_sink: Option<u32>,
    pub default_source: Option<u32>,
}

pub fn run(updates: Sender<Update>) {
    pw::init();
    loop {
        if let Err(error) = follow(&updates) {
            tracing::info!("no PipeWire: {error}");
        }
        if updates.send(Update::Audio(Graph::default())).is_err() {
            return;
        }
        std::thread::sleep(RETRY);
    }
}

/// A node with its proxy and what it last said.
struct Tracked {
    node: AudioNode,
    /// The node's full properties (its info, not the registry's few) leave it out: a smart filter, a stream
    /// without an application name. Unknown until the info came.
    shown: Option<bool>,
    channel_volumes: Option<Vec<f32>>,
    muted: Option<bool>,
    _proxy: Node,
    _listener: NodeListener,
}

#[derive(Default)]
struct State {
    /// In the order they appeared.
    nodes: Vec<(u32, Tracked)>,
    /// `node.name` of the default output and microphone.
    default_sink: Option<String>,
    default_source: Option<String>,
    metadata: Option<(Metadata, MetadataListener)>,
    sent: Option<Graph>,
}

impl State {
    fn graph(&self) -> Graph {
        let id_of = |name: &Option<String>| {
            let name = name.as_ref()?;
            self.nodes.iter().find(|(_, t)| &t.node.name == name).map(|(id, _)| *id)
        };
        let nodes = self
            .nodes
            .iter()
            .filter(|(_, t)| t.shown == Some(true))
            .map(|(_, t)| {
                let audio = t.channel_volumes.as_deref().and_then(volume).zip(t.muted).map(|(volume, muted)| Audio { volume, muted });
                AudioNode { audio, ..t.node.clone() }
            })
            .collect();
        Graph { nodes, default_sink: id_of(&self.default_sink), default_source: id_of(&self.default_source) }
    }

    fn report(&mut self, updates: &Sender<Update>) {
        let graph = self.graph();
        if self.sent.as_ref() != Some(&graph) {
            self.sent = Some(graph.clone());
            let _ = updates.send(Update::Audio(graph));
        }
    }
}

/// What a node of `media.class` may be to the panel.
fn candidate(class: &str) -> Option<NodeKind> {
    match class {
        "Audio/Sink" => Some(NodeKind::Sink),
        class if class.starts_with("Audio/Source") => Some(NodeKind::Source),
        "Stream/Output/Audio" => Some(NodeKind::Stream),
        _ => None,
    }
}

/// Whether a candidate of `kind` shows, by its full properties.
fn shown(kind: NodeKind, smart_filter: bool, application: &str) -> bool {
    match kind {
        NodeKind::Sink => true,
        NodeKind::Source => !smart_filter,
        NodeKind::Stream => !application.is_empty(),
    }
}

/// The volume on the cubic scale of `wpctl` and Quickshell: the mean of the channels' cube roots.
pub fn volume(channels: &[f32]) -> Option<f32> {
    if channels.is_empty() {
        return None;
    }
    Some(channels.iter().map(|v| v.max(0.0).cbrt()).sum::<f32>() / channels.len() as f32)
}

/// Channel volumes and mute of a `Props` pod, as far as it has them.
fn node_props(pod: &Pod) -> (Option<Vec<f32>>, Option<bool>) {
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes()) else { return (None, None) };
    let mut found = (None, None);
    for property in object.properties {
        match (property.key, property.value) {
            (CHANNEL_VOLUMES, Value::ValueArray(ValueArray::Float(volumes))) => found.0 = Some(volumes),
            (MUTE, Value::Bool(muted)) => found.1 = Some(muted),
            _ => {}
        }
    }
    found
}

/// The node name in a `default.audio.sink` value (`{"name":"alsa_output…"}`).
fn default_name(value: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(value).ok()?;
    value["name"].as_str().map(str::to_owned)
}

/// Follow the graph until the connection to PipeWire ends.
fn follow(updates: &Sender<Update>) -> Result<(), pw::Error> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let state = Rc::new(RefCell::new(State::default()));

    // The server went away (restarted): start over.
    let quit = mainloop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _seq, _res, message| {
            if id == pw::core::PW_ID_CORE {
                tracing::info!("PipeWire connection ended: {message}");
                if let Some(mainloop) = quit.upgrade() {
                    mainloop.quit();
                }
            }
        })
        .register();

    let weak_registry = registry.downgrade();
    let weak_state = Rc::downgrade(&state);
    let sender = updates.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let (Some(registry), Some(state)) = (weak_registry.upgrade(), weak_state.upgrade()) else { return };
            let props = global.props.as_ref();
            let prop = |key: &str| props.and_then(|p| p.get(key)).map(str::to_owned).unwrap_or_default();
            match global.type_ {
                ObjectType::Node => {
                    let Some(kind) = candidate(&prop("media.class")) else { return };
                    let Ok(proxy) = registry.bind::<Node, _>(global) else { return };
                    let id = global.id;
                    let weak = Rc::downgrade(&state);
                    let info_weak = Rc::downgrade(&state);
                    let (param_sender, info_sender) = (sender.clone(), sender.clone());
                    let listener = proxy
                        .add_listener_local()
                        .info(move |info| {
                            let Some(state) = info_weak.upgrade() else { return };
                            // Later info events carry only what changed: properties only when they did.
                            if !info.change_mask().contains(NodeChangeMask::PROPS) {
                                return;
                            }
                            let Some(props) = info.props() else { return };
                            let get = |key: &str| props.get(key).unwrap_or_default().to_owned();
                            let mut state = state.borrow_mut();
                            if let Some((_, tracked)) = state.nodes.iter_mut().find(|(n, _)| *n == id) {
                                let node = &mut tracked.node;
                                node.name = get("node.name");
                                node.description = get("node.description");
                                node.application = get("application.name");
                                tracked.shown = Some(shown(node.kind, get("filter.smart") == "true", &node.application));
                            }
                            state.report(&info_sender);
                        })
                        .param(move |_seq, param, _index, _next, pod| {
                            let (Some(state), Some(pod), ParamType::Props) = (weak.upgrade(), pod, param) else { return };
                            let (volumes, muted) = node_props(pod);
                            let mut state = state.borrow_mut();
                            if let Some((_, tracked)) = state.nodes.iter_mut().find(|(n, _)| *n == id) {
                                tracked.channel_volumes = volumes.or(tracked.channel_volumes.take());
                                tracked.muted = muted.or(tracked.muted);
                            }
                            state.report(&param_sender);
                        })
                        .register();
                    proxy.subscribe_params(&[ParamType::Props]);
                    let node = AudioNode { id, kind, ..AudioNode::default() };
                    let tracked = Tracked { node, shown: None, channel_volumes: None, muted: None, _proxy: proxy, _listener: listener };
                    let mut state = state.borrow_mut();
                    state.nodes.push((id, tracked));
                    state.report(&sender);
                }
                ObjectType::Metadata if prop("metadata.name") == "default" => {
                    let Ok(metadata) = registry.bind::<Metadata, _>(global) else { return };
                    let weak = Rc::downgrade(&state);
                    let sender = sender.clone();
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _type, value| {
                            if let Some(state) = weak.upgrade() {
                                let mut state = state.borrow_mut();
                                match key {
                                    // Every key of the default's subject (0) went. The same for another subject is its
                                    // object going away, as a stream's `target.node` does when the stream ends.
                                    None if subject == 0 => {
                                        state.default_sink = None;
                                        state.default_source = None;
                                    }
                                    Some("default.audio.sink") if subject == 0 => state.default_sink = value.and_then(default_name),
                                    Some("default.audio.source") if subject == 0 => state.default_source = value.and_then(default_name),
                                    _ => return 0,
                                }
                                state.report(&sender);
                            }
                            0
                        })
                        .register();
                    state.borrow_mut().metadata = Some((metadata, listener));
                }
                _ => {}
            }
        })
        .global_remove({
            let weak = Rc::downgrade(&state);
            let sender = updates.clone();
            move |id| {
                if let Some(state) = weak.upgrade() {
                    let mut state = state.borrow_mut();
                    let before = state.nodes.len();
                    state.nodes.retain(|(n, _)| *n != id);
                    if state.nodes.len() != before {
                        state.report(&sender);
                    }
                }
            }
        })
        .register();

    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_is_on_the_cubic_scale() {
        // 0.8 in wpctl and the bar is 0.512 in PipeWire's channels.
        let volume = volume(&[0.512, 0.512]).unwrap();
        assert!((volume - 0.8).abs() < 1e-4, "{volume}");
        assert_eq!(super::volume(&[]), None);
        assert_eq!(super::volume(&[-0.1]), Some(0.0));
    }

    #[test]
    fn only_outputs_real_microphones_and_named_streams_count() {
        assert_eq!(candidate("Audio/Sink"), Some(NodeKind::Sink));
        assert_eq!(candidate("Audio/Source/Virtual"), Some(NodeKind::Source));
        assert_eq!(candidate("Video/Source"), None);
        assert!(shown(NodeKind::Source, false, ""));
        assert!(!shown(NodeKind::Source, true, ""));
        assert!(shown(NodeKind::Stream, false, "Firefox"));
        assert!(!shown(NodeKind::Stream, false, ""));
    }

    #[test]
    fn the_default_sink_is_named_in_json() {
        assert_eq!(default_name(r#"{ "name": "alsa_output.pci-0000_00_1f.3" }"#).as_deref(), Some("alsa_output.pci-0000_00_1f.3"));
        assert_eq!(default_name(r#"{"name":"test-sink"}"#).as_deref(), Some("test-sink"));
        assert_eq!(default_name("{}"), None);
    }
}
