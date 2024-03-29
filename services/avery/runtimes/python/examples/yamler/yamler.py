"""
Tests python dependency packaging
"""

import firm  # type: ignore # pylint: disable=import-error
import yaml


def main() -> None:
    """
    Tests python wheel packaging
    """
    thing = yaml.load(firm.get_input("yaml"), Loader=yaml.Loader)
    key = firm.get_input("yamlkey")
    print("Hello I am Yamler")
    firm.set_output("utputt", [str(thing.get(key, "Not found"))])


if __name__ == "__main__":
    main()
