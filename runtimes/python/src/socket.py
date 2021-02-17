import wasi_socket

class socket:
    def __init__(self, *args, **kwargs):
        self.wasi_socket = wasi_socket.new_socket()
        self.closed = False

    def connect(self, address):
        wasi_socket.connect(self.wasi_socket, address)

    def send(self, data, flags=None):
        wasi_socket.send(self.wasi_socket, data, flags)

    def recv(self, bufsize, flags=None):
        wasi_socket.recv(self.wasi_socket, bufsize, flags)

    def close(self):
        self.closed = True

    def __getattr__(cls, key):
        raise AttributeError(
            f'"{key}" is not implemented for WASI sockets. '
            "It needs to be implemented in the python WASI runtime"
        )
