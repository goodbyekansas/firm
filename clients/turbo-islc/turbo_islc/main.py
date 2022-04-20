"""Entry point for the Turbo ISL Compiler."""
from __future__ import annotations

import argparse
import sys
import typing

import pyparsing as pp

from turbo_islc.ast import Module, SyntaxNode, lex, parse
from turbo_islc.error import TurboException, TurboSyntaxError
from turbo_islc.target import Target
from turbo_islc.targets.rust_wasmtime import rust_wasmtime


def write_output(
    output_stream: typing.TextIO,
    ast: typing.Iterator[SyntaxNode],
    target: typing.Type[Target],
) -> None:
    """Write the output using the selected target.

    Args:
        output_stream (TextIO): Destination for compiler output.
        ast: (Iterator[SyntaxNode]): Abstract syntax tree to generate code for.
        target: (Target): Target for code generation,
                          i.e. what generation method to use.

    """

    tgt = target()

    for module in ast:
        if isinstance(module, Module):
            # TODO: Should be real Namespace object parsed from command line
            for fragment in tgt.generate(module, argparse.Namespace()):
                output_stream.write(fragment)


def discover_targets() -> list[typing.Type[Target]]:
    """Discover supported targets.

    Discover and generate a list of all supported targets. Currently only supports
    built-in targets but might support plugins later.

    Returns:
        list[Target]: A list of supported targets

    """
    targets: list[typing.Type[Target]] = []
    # builtin targets
    targets.append(rust_wasmtime.RustWasmtime)

    # TODO: some plugin thing, pkg_resources entry points?

    return targets


def main() -> int:
    """Run the main function of turbo isl.

    Returns:
        int: Use this as your exit code!

    """
    targets = discover_targets()
    parser = argparse.ArgumentParser(description="Turbo ISL Compiler")
    parser.add_argument(
        "in_file",
        type=argparse.FileType("r"),
        nargs="?",
        default=sys.stdin,
        help="Input .tisl file, if omitted read from stdin",
    )
    parser.add_argument(
        "out_file",
        nargs="?",
        type=argparse.FileType("w"),
        default=sys.stderr,
        help=(
            "File path to save the compiler output in. "
            "If not given, output is printed to stderr"
        ),
    )

    target_dict = dict(map(lambda t: (t.name, t), targets))
    parser.add_argument(
        "-t",
        "--target",
        choices=target_dict.keys(),
        default=list(target_dict.keys())[0],
        help="Target for code generation",
    )
    args = parser.parse_args()
    try:
        ast = map(parse, lex(args.in_file.read()))
    except pp.ParseException as err:
        print(
            TurboSyntaxError(
                err.msg,
                filename=args.in_file,
                lineno=err.lineno,
                column=err.column,
                line=err.line,
            )
        )
        return 1
    except pp.ParseSyntaxException as err:
        print(
            TurboSyntaxError(
                err.msg,
                filename=args.in_file,
                lineno=err.lineno,
                column=err.column,
                line=err.line,
            )
        )
        return 1
    except TurboSyntaxError as err:
        print(
            TurboSyntaxError(
                message=err.message,
                filename=args.in_file,
                lineno=err.lineno,
                column=err.column,
                line=err.line,
            )
        )
        return 1

    try:
        target = target_dict[args.target]
        write_output(args.out_file, ast, target)
    except TurboException as err:
        print(TurboException(message=err.message, filename=args.in_file))
        return 3

    return 0


if __name__ == "__main__":
    sys.exit(main())
