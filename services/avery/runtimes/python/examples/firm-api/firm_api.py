import typing
import os.path

from collections.abc import Iterable
import firm

def main()->None:
    inputs = ["str_input", "int_input", "float_input", "bool_input", "bytes_input"]
    for input in inputs:
        print(f"{input} is: {firm.get_input(input)}")

    inputs = ["str_list_input", "int_list_input", "float_list_input", "bool_list_input"]
    for input in inputs:
        print(f"{input} is: [{', '.join(map(str, firm.get_input_stream(input)))}]")

    outputs = {
        "str_output": ["i", "am", "output", "me too"],
        "int_output": [1, 3, 2, 4],
        "float_output": [1.4, 13.37, 1],
        "bool_output": [False, True, False, False],
        "bytes_output": bytes([10, 11]),
    }

    for key, value in outputs.items():
        firm.set_output(key, value)

    host_os = firm.get_host_os()
    print(f"Host Os: {host_os}")

    print(f'Windows host path exists (C:): {firm.host_path_exists("C:")}')
    print(f'Unix host path exists (/tmp): {firm.host_path_exists("/tmp")}')

    print("Starting echo process")
    pid = firm.start_host_process("echo", ["Mega Rune"])
    print(f"Process PID: {pid}")

    if host_os == "windows":
        firm.start_host_process("cmd", ["/C", "echo %MY_ENVIRONMENT%"], { "MY_ENVIRONMENT": "I R in windows! 🎎"} )
        print(f'Exit 5 returned: {firm.run_host_process("cmd", ["/C", "exit 5"])}')
    else:
        firm.start_host_process("sh", ["-c", "echo $MY_ENVIRONMENT"], { "MY_ENVIRONMENT": "I R in unix! 🧔"} )
        print(f'Exit 5 returned: {firm.run_host_process("sh", ["-c", "exit 5"])}')

    data_path = firm.map_attachment("data")
    print(f"Data path: {data_path}")
    print("Data content:")
    with open(data_path, "r") as data_file:
        print(data_file.read())

    compressed_data_path = firm.map_attachment("compressed_data", unpack=True)
    print(f"Compressed data path: {compressed_data_path}")
    print("Compressed data content:")
    with open(os.path.join(compressed_data_path, "much_data.dat"), "r") as compressed_data_file:
        print(compressed_data_file.read())

def main_with_error() -> None:
    print("Setting error")
    firm.set_error("Everything actually went fine")

if __name__ == "__main__":
    main()