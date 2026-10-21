#!/usr/bin/env python3
"""A stand-in for greetd at the socket path given first: the user named second logs in with the password given
third. Requests go to the log file given fourth, one line each (passwords left out), so a test sees what the
greeter asked for and which session it started."""
import json
import os
import socket
import struct
import sys

path, user, password, log_path = sys.argv[1:5]
log = open(log_path, "a", buffering=1)


def read(connection):
    header = connection.recv(4, socket.MSG_WAITALL)
    if len(header) < 4:
        return None
    (length,) = struct.unpack("=i", header)
    return json.loads(connection.recv(length, socket.MSG_WAITALL))


def write(connection, message):
    data = json.dumps(message).encode()
    connection.sendall(struct.pack("=i", len(data)) + data)


server = socket.socket(socket.AF_UNIX)
server.bind(path)
server.listen()
log.write("ready\n")
while True:
    connection, _ = server.accept()
    session_user = None
    while (request := read(connection)) is not None:
        kind = request["type"]
        if kind == "create_session":
            session_user = request["username"]
            log.write(f"create_session {session_user}\n")
            write(connection, {"type": "auth_message", "auth_message_type": "secret", "auth_message": "Password:"})
        elif kind == "post_auth_message_response":
            right = session_user == user and request.get("response") == password
            log.write(f"response {'right' if right else 'wrong'}\n")
            if right:
                write(connection, {"type": "success"})
            else:
                # Like greetd: the failed session is over, its worker gone.
                session_user = None
                write(connection, {"type": "error", "error_type": "auth_error", "description": "pam_authenticate: AUTH_ERR"})
        elif kind == "start_session":
            log.write("start_session " + " ".join(request["cmd"]) + "\n")
            write(connection, {"type": "success"})
        elif kind == "cancel_session":
            log.write("cancel_session\n")
            # greetd answers a cancel without a session (after a failed one) with an error.
            if session_user is None:
                write(connection, {"type": "error", "error_type": "error",
                                   "description": "unable to send message: Connection refused (os error 111)"})
            else:
                write(connection, {"type": "success"})
    connection.close()
