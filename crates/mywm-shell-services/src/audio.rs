//! The default output's volume and mute, as PipeWire reports them: every `Audio/Sink` node is bound and its
//! `Props` followed, and the `default` metadata's `default.audio.sink` says which of them is the default.
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::mpsc::Sender};

use mywm_shell_protocol::Audio;
use pipewire as pw;
use pw::{
    metadata::{Metadata, MetadataListener},
    node::{Node, NodeListener},
    spa::{
        param::ParamType,
        pod::{Pod, Value, ValueArray, deserialize::PodDeserializer},
    },
    types::ObjectType,
};

use crate::{RETRY, Update};

const CHANNEL_VOLUMES: u32 = pw::spa::sys::SPA_PROP_channelVolumes;
const MUTE: u32 = pw::spa::sys::SPA_PROP_mute;

pub fn run(updates: Sender<Update>) {
    pw::init();
    loop {
        if let Err(error) = follow(&updates) {
            tracing::info!("no PipeWire: {error}");
        }
        if updates.send(Update::Audio(None)).is_err() {
            return;
        }
        std::thread::sleep(RETRY);
    }
}

/// A sink with its proxy and what it last said.
struct Sink {
    name: String,
    channel_volumes: Option<Vec<f32>>,
    muted: Option<bool>,
    _node: Node,
    _listener: NodeListener,
}

#[derive(Default)]
struct Graph {
    sinks: HashMap<u32, Sink>,
    /// `node.name` of the default output.
    default: Option<String>,
    metadata: Option<(Metadata, MetadataListener)>,
    sent: Option<Option<Audio>>,
}

impl Graph {
    fn current(&self) -> Option<Audio> {
        let name = self.default.as_ref()?;
        let sink = self.sinks.values().find(|sink| &sink.name == name)?;
        Some(Audio { volume: volume(sink.channel_volumes.as_deref()?)?, muted: sink.muted? })
    }

    fn report(&mut self, updates: &Sender<Update>) {
        let current = self.current();
        if self.sent != Some(current) {
            self.sent = Some(current);
            let _ = updates.send(Update::Audio(current));
        }
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
fn sink_props(pod: &Pod) -> (Option<Vec<f32>>, Option<bool>) {
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
    let graph = Rc::new(RefCell::new(Graph::default()));

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
    let weak_graph = Rc::downgrade(&graph);
    let sender = updates.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let (Some(registry), Some(graph)) = (weak_registry.upgrade(), weak_graph.upgrade()) else { return };
            let props = global.props.as_ref();
            let prop = |key: &str| props.and_then(|p| p.get(key)).map(str::to_owned);
            match global.type_ {
                ObjectType::Node if prop("media.class").as_deref() == Some("Audio/Sink") => {
                    let Ok(node) = registry.bind::<Node, _>(global) else { return };
                    let id = global.id;
                    let weak = Rc::downgrade(&graph);
                    let sender = sender.clone();
                    let listener = node
                        .add_listener_local()
                        .param(move |_seq, kind, _index, _next, pod| {
                            let (Some(graph), Some(pod)) = (weak.upgrade(), pod) else { return };
                            if kind != ParamType::Props {
                                return;
                            }
                            let (volumes, muted) = sink_props(pod);
                            let mut graph = graph.borrow_mut();
                            if let Some(sink) = graph.sinks.get_mut(&id) {
                                sink.channel_volumes = volumes.or(sink.channel_volumes.take());
                                sink.muted = muted.or(sink.muted);
                            }
                            graph.report(&sender);
                        })
                        .register();
                    node.subscribe_params(&[ParamType::Props]);
                    let name = prop("node.name").unwrap_or_default();
                    let sink = Sink { name, channel_volumes: None, muted: None, _node: node, _listener: listener };
                    graph.borrow_mut().sinks.insert(id, sink);
                }
                ObjectType::Metadata if prop("metadata.name").as_deref() == Some("default") => {
                    let Ok(metadata) = registry.bind::<Metadata, _>(global) else { return };
                    let weak = Rc::downgrade(&graph);
                    let sender = sender.clone();
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _type, value| {
                            if let Some(graph) = weak.upgrade() {
                                let mut graph = graph.borrow_mut();
                                match key {
                                    // Every key went (the metadata was cleared).
                                    None => graph.default = None,
                                    Some("default.audio.sink") if subject == 0 => graph.default = value.and_then(default_name),
                                    Some(_) => return 0,
                                }
                                graph.report(&sender);
                            }
                            0
                        })
                        .register();
                    graph.borrow_mut().metadata = Some((metadata, listener));
                }
                _ => {}
            }
        })
        .global_remove({
            let weak = Rc::downgrade(&graph);
            let sender = updates.clone();
            move |id| {
                if let Some(graph) = weak.upgrade() {
                    let mut graph = graph.borrow_mut();
                    if graph.sinks.remove(&id).is_some() {
                        graph.report(&sender);
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
    fn the_default_sink_is_named_in_json() {
        assert_eq!(default_name(r#"{ "name": "alsa_output.pci-0000_00_1f.3" }"#).as_deref(), Some("alsa_output.pci-0000_00_1f.3"));
        assert_eq!(default_name(r#"{"name":"test-sink"}"#).as_deref(), Some("test-sink"));
        assert_eq!(default_name("{}"), None);
    }
}
