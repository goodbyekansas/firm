""" firm networking example """
import socket

import firm  # noqa


def main() -> None:
    """ ports and sockets """
    port = firm.get_input("port")
    sock = socket.socket()
    sock.connect(("localhost", port))

    sock.send(b"hello network!\n")

    try:
        sock.snedmesage()  # type: ignore
    except AttributeError as error:
        print(f"Got the expected attribute error: {error}")
