def __getattr__(key):
    raise AttributeError(
        f'"{key}" is not implemented for the WASI select module. '
        "It needs to be implemented in the Python WASI runtime."
    )
