import socket

import firm


def main() -> None:
    port = firm.get_input("port")
    s = socket.socket()
    s.connect(("localhost", port))

    s.send(b"hello network!\n")

    try:
        s.snedmesage()
    except AttributeError as e:
        print(f"Got the expected attribute error: {e}")

