"""Local MySQL protocol fixture. Uses ephemeral TLS certificates and no database."""
import json
import pathlib
import socket
import ssl
import struct
import subprocess
import sys
import threading

root = pathlib.Path(sys.argv[1])
log_lock = threading.Lock()
cancelled = threading.Event()
next_id = 40
id_lock = threading.Lock()


def log(text):
    with log_lock:
        with (root / "queries.log").open("a") as output:
            output.write(text + "\n")


def openssl(*args):
    subprocess.run(["openssl", *args], check=True, stdout=subprocess.DEVNULL,
                   stderr=subprocess.PIPE)


(root / "ca.cnf").write_text(
    "[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n"
    "[dn]\nCN=Org Studio Test CA\n[ca]\nbasicConstraints=critical,CA:TRUE\n"
    "keyUsage=critical,keyCertSign,cRLSign\n"
)
openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes",
        "-keyout", str(root / "ca.key"), "-out", str(root / "ca.crt"),
        "-days", "1", "-config", str(root / "ca.cnf"))
openssl("req", "-new", "-newkey", "rsa:2048", "-nodes",
        "-keyout", str(root / "server.key"), "-out", str(root / "server.csr"),
        "-subj", "/CN=localhost")
(root / "server.ext").write_text(
    "basicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\n"
    "extendedKeyUsage=serverAuth\nsubjectAltName="
    + (sys.argv[2] if len(sys.argv) > 2 else "DNS:localhost,IP:127.0.0.1") + "\n"
)
openssl("x509", "-req", "-in", str(root / "server.csr"),
        "-CA", str(root / "ca.crt"), "-CAkey", str(root / "ca.key"),
        "-CAcreateserial", "-out", str(root / "server.crt"), "-days", "1",
        "-extfile", str(root / "server.ext"))
tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
tls.load_cert_chain(root / "server.crt", root / "server.key")


def packet(conn, payload, sequence):
    conn.sendall(len(payload).to_bytes(3, "little") + bytes([sequence % 256]) + payload)


def read_exact(conn, size):
    data = b""
    while len(data) < size:
        part = conn.recv(size - len(data))
        if not part:
            raise EOFError()
        data += part
    return data


def receive(conn):
    header = read_exact(conn, 4)
    return read_exact(conn, int.from_bytes(header[:3], "little")), header[3]


def enc(value):
    if value is None:
        return b"\xfb"
    if isinstance(value, str):
        value = value.encode()
    if len(value) < 251:
        return bytes([len(value)]) + value
    return b"\xfc" + struct.pack("<H", len(value)) + value


def column(name, kind=253, flags=0):
    return (enc("def") + enc("analytics") + enc("") + enc("") + enc(name) + enc(name)
            + b"\x0c" + struct.pack("<HI", 45, 1024) + bytes([kind])
            + struct.pack("<H", flags) + b"\x00\x00\x00")


def ok(conn, sequence=1):
    packet(conn, b"\x00\x00\x00\x02\x00\x00\x00", sequence)


def eof(conn, sequence):
    packet(conn, b"\xfe\x00\x00\x02\x00", sequence)


def error(conn, number=1064, message="Fixture syntax error"):
    packet(conn, b"\xff" + struct.pack("<H", number) + b"#42000" + message.encode(), 1)


columns = [("code", 253, 0), ("amount", 8, 32), ("total", 246, 0),
           ("note", 253, 0), ("message", 253, 0)]
values = ["00123", "18446744073709551615", "1234567890.123456789012345678",
          None, "a|b\n中文"]


def result(conn, specs, rows, binary=False):
    sequence = 1
    packet(conn, bytes([len(specs)]), sequence)
    for name, kind, flags in specs:
        sequence += 1
        packet(conn, column(name, kind, flags), sequence)
    sequence += 1
    eof(conn, sequence)
    for row in rows:
        sequence += 1
        if binary:
            payload = b"\x00\x00"
            for value, (_, kind, _) in zip(row, specs):
                payload += struct.pack("<Q", int(value)) if kind == 8 else enc(value)
        else:
            payload = b"".join(enc(value) for value in row)
        packet(conn, payload, sequence)
    eof(conn, sequence + 1)


def client(conn):
    global next_id
    with id_lock:
        next_id += 1
        connection_id = next_id
    try:
        capabilities = 1 | 4 | 8 | 512 | 2048 | 8192 | 32768 | 131072 | 262144 | 524288
        handshake = (b"\x0a8.0.36-org-studio-test\x00" + struct.pack("<I", connection_id)
                     + b"12345678\x00" + struct.pack("<H", capabilities & 65535)
                     + b"\x2d\x02\x00" + struct.pack("<H", capabilities >> 16)
                     + b"\x15" + bytes(10) + b"abcdefghijkL\x00mysql_native_password\x00")
        packet(conn, handshake, 0)
        auth, sequence = receive(conn)
        if len(auth) == 32:
            conn = tls.wrap_socket(conn, server_side=True)
            auth, sequence = receive(conn)
            log("TLS")
        ok(conn, sequence + 1)
        statement_id = 0
        while True:
            data, _ = receive(conn)
            command = data[0]
            if command == 1:
                break
            if command == 14:
                ok(conn)
            elif command == 22:
                query = data[1:].decode()
                log("PREPARE " + query)
                if "SYNTAX_ERROR" in query:
                    error(conn)
                    continue
                statement_id += 1
                specs = [("connection_id", 8, 32), ("sql_mode", 253, 0)] if "CONNECTION_ID()" in query else columns
                if query.startswith("EXPLAIN"):
                    specs = []
                packet(conn, b"\x00" + struct.pack("<IHHBH", statement_id, len(specs), 0, 0, 0), 1)
                for sequence, spec in enumerate(specs, 2):
                    packet(conn, column(*spec), sequence)
                if specs:
                    eof(conn, len(specs) + 2)
            elif command == 23:
                mode = sys.argv[3] if len(sys.argv) > 3 else ""
                result(conn, [("connection_id", 8, 32), ("sql_mode", 253, 0)],
                       [[str(connection_id), mode]], binary=True)
            elif command == 25:
                pass
            elif command == 3:
                query = data[1:].decode()
                log("QUERY " + query)
                if query.startswith("SELECT SLEEP"):
                    cancelled.wait(15)
                    error(conn, 1317, "Query execution was interrupted")
                elif query.startswith("KILL QUERY"):
                    cancelled.set()
                    ok(conn)
                elif query.startswith("EXPLAIN"):
                    result(conn, [("id", 8, 32), ("select_type", 253, 0),
                                  ("table", 253, 0), ("rows", 8, 32), ("Extra", 253, 0)],
                           [["1", "SIMPLE", "orders", "12", "Using where"]])
                elif query.startswith("SELECT") or query.startswith("WITH"):
                    rows = [] if "WHERE 1=0" in query else [values]
                    if "MANY_ROWS" in query:
                        rows = [values] * 1002
                    result(conn, columns, rows)
                else:
                    ok(conn)
            else:
                raise RuntimeError(f"Unexpected MySQL command: {command}")
    except (EOFError, ConnectionError, ssl.SSLError):
        pass
    except Exception as exception:
        log("ERROR " + repr(exception))
    finally:
        conn.close()


server = socket.socket()
server.bind(("127.0.0.1", 0))
server.listen()
print(json.dumps({"port": server.getsockname()[1]}), flush=True)
while True:
    connection, _ = server.accept()
    threading.Thread(target=client, args=(connection,), daemon=True).start()
