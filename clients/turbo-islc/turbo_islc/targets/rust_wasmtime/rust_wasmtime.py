"""Rust Wasmtime target.

Generates host side bindings for the API to use with the wasmtime library.
"""

import argparse
import itertools
import textwrap
import typing
from dataclasses import dataclass

import jinja2

import turbo_islc.jinja as jinja_common
from turbo_islc import ast
from turbo_islc.error import TurboException
from turbo_islc.target import Target


def to_safe_snakecase(word: str) -> str:
    return (
        "r#" if word in ["type", "self", "ref", "return"] else ""
    ) + jinja_common.to_snakecase(word)


class Lifetime:  # pylint: disable=too-few-public-methods
    """Helpers for generating Rust lifetime declarations."""

    @staticmethod
    def struct(
        fields: typing.Union[
            dict[str, ast.Field], dict[str, ast.FunctionReturnValue], dict
        ],
        lifetime_name: str,
        fmt: str = "<%s>",
    ) -> str:
        """Generate a lifetime declaration for a struct.

        Will check if any of the fields is a reference to something, and in that case
        generate a lifetime declaration suitable for a Rust struct.

        Args:
            fields: (dict): Fields for the structure.
            lifetime_name (str): Name to use for the lifetime if needed.
            fmt (str): Format for the lifetime declaration.
                       Should contain a single '%s'.

        Returns:
            str: A lifetime declaration to use with a Rust struct.

        """
        if any(map(lambda f: f.is_reference(), fields.values())):
            return fmt % (f"'{lifetime_name}")

        return ""


class Types:
    """Helpers for generating Rust type declarations.

    Attributes:
        abi_size_type (str): The rust type used to marshal pointers and array sizes
        lifetime (Lifetime): Lifetime helpers instance (used to generate struct names)
        rust_types (dict[str, str]): Mapping of TISL types to Rust types
        rust_ref_types (dict[str, str]): Mapping of TISL types to Rust reference types
        abi_types_in (dict[str, str]): Mapping of TISL types to Rust ABI types when
                                       used as inputs
        abi_types (dict[str, str]): Mapping of TISL types to Rust ABI types
                                    in struct and list marshaling

    """

    abi_size_type: str = "i64"
    lifetime: Lifetime

    rust_types: dict = {
        "int": "i64",
        "float": "f64",
        "bool": "bool",
        "string": "String",
        "bytes": "u8",
    }

    rust_ref_types: dict = {
        "int": "i64",
        "float": "f64",
        "bool": "bool",
        "string": "str",
        "bytes": "u8",
    }

    abi_types_in: dict = {
        "int": "i64",
        "float": "f64",
        "bool": "i32",
        "string": abi_size_type,
    }

    abi_types: dict = {
        "int": "i64",
        "float": "f64",
        "bool": "u8",
        "string": abi_size_type,
        "bytes": "u8",
    }

    def __init__(self, lifetime: Lifetime, abi_size: int = 64) -> None:
        """Initialize type helper.

        Args:
            lifetime (Lifetime): A Lifetime helper object.
            abi_size (int): The size to use for pointers and sizes when marshaling
                            (can only be 32 or 64).
        """
        self.abi_size_type = f"i{str(abi_size if abi_size in [32, 64] else 64)}"
        self.lifetime = lifetime

    def _struct_name(
        self,
        name: str,
        fields: typing.Union[
            dict[str, ast.Field], dict[str, ast.FunctionReturnValue], dict
        ],
        lifetime_name: str,
    ) -> str:
        """Generate an appropriate name for a Rust struct."""
        return (
            f"{jinja_common.to_camelcase(name)}"
            f"{self.lifetime.struct(fields, lifetime_name=lifetime_name)}"
        )

    def _to_rust_types(
        self,
        arg: typing.Union[ast.FunctionArgument, ast.FunctionReturnValue, ast.Field],
        lifetime_name: str = "_",
    ) -> typing.Tuple[str, str]:
        if arg.is_record():
            record = arg.as_record()
            type_name = self._struct_name(
                arg.type_name(),
                record.fields if record else {},
                lifetime_name=lifetime_name,
            )
            return (type_name, type_name)
        elif arg.is_enum():
            return (
                jinja_common.to_camelcase(arg.type_name()),
                jinja_common.to_camelcase(arg.type_name()),
            )
        try:
            return (
                self.rust_types[arg.type_name()],
                self.rust_ref_types[arg.type_name()],
            )
        except KeyError as kerr:
            raise TurboException(
                message=f"Failed to lookup rust type: {arg.type_name()}"
            ) from kerr

    def trait_input(self, arg: ast.FunctionArgument, lifetime_name: str) -> str:
        """Convert a function argument to a trait input.

        Args:
            arg: (FunctionArgument): function argument to convert.
            lifetime_name: (str): Lifetime name to use if needed (it is a reference).

        Returns:
            str: A string containing a trait input declaration.
        """

        # all of these cases should yield a reference when in the trait input position
        is_reference = (
            arg.is_reference()
            or arg.is_list()
            or arg.type_name() == "string"
            or arg.is_record()
        )
        type_name, ref_type_name = self._to_rust_types(arg, lifetime_name=lifetime_name)

        if arg.is_list():
            if is_reference:
                type_name = f"[{type_name}]"
            else:
                type_name = f"Vec<{type_name}>"
        else:
            if is_reference:
                type_name = ref_type_name

        reference = f"&'{lifetime_name}" if is_reference else ""
        return f"{to_safe_snakecase(arg.name)}: {reference} {type_name}"

    def _single_trait_output(self, return_value: ast.FunctionReturnValue) -> str:
        type_name, ref_type_name = self._to_rust_types(return_value)

        if return_value.is_list():
            if return_value.is_reference():
                type_name = f"[{type_name}]"
            else:
                type_name = f"Vec<{type_name}>"
        elif return_value.is_reference():
            type_name = ref_type_name

        return f"{'&' if return_value.is_reference() else ''}{type_name}"

    def trait_output(self, function: ast.Function) -> str:
        """Extract a trait output declaration from a function definition.

        Args:
            function (Function): Function to extract output declaration from

        Returns:
            str: String containing a Rust trait output type

        """
        if not function.has_return_values():
            return "()"

        if function.is_multi_return():
            return self._struct_name(
                f"{function.name}-result", function.return_values, lifetime_name="_"
            )

        return_value = function.get_first_return_value()
        if return_value is None:
            raise TurboException(
                message=f"Failed to get return value for function {function.name}"
            )

        return self._single_trait_output(return_value)

    def struct_field(
        self, field: ast.Field, lifetime_name: str, pub: bool = True
    ) -> str:
        """Convert a record field to a Rust struct field declaration.

        Args:
            field (Field): record field to convert.
            lifetime_name (str): Name to use if a lifetime is needed for the field.
            pub (bool): True if the field should be declared as public
                        (True is default).

        Returns:
            str: A string containing a Rust struct field declaration.
        """
        type_name, ref_type_name = self._to_rust_types(
            field, lifetime_name=lifetime_name
        )

        if field.is_list():
            if field.is_reference():
                type_name = f"[{type_name}]"
            else:
                type_name = f"Vec<{type_name}>"
        else:
            if field.is_reference():
                type_name = ref_type_name

        reference = f"&'{lifetime_name} " if field.is_reference() else ""
        return (
            f"{'pub ' if pub else ''}{to_safe_snakecase(field.name)}"
            f": {reference}{type_name},"
        )

    def wrapper_in(self, arg: ast.FunctionArgument) -> str:
        """Convert a function argument to a wrapper function arg declaration.

        A wrapper function is a WASM function that converts inputs and outputs between
        Rust native types and WASM.

        Args:
            arg (FunctionArgument): The function argument to convert.

        Returns:
            str: A string containing a Rust input declaration for a wrapper function.

        """
        if arg.is_list():
            type_name = self.abi_size_type
        elif arg.is_record():
            type_name = self.abi_size_type
        elif arg.is_enum():
            type_name = "i32"
        else:
            type_name = self.abi_types_in[arg.type_name()]

        decl = f"{to_safe_snakecase(arg.name)}: {type_name},"
        if arg.is_list():
            decl += f"\n{to_safe_snakecase(arg.name)}_len:" f" {self.abi_size_type}"

        return decl

    def wrapper_out(self, arg: ast.FunctionReturnValue) -> str:
        """Convert a return value to a wrapper function arg declaration.

        A wrapper function is a WASM function that converts inputs and outputs between
        Rust native types and WASM.

        Args:
            arg (FunctionArgument): The function argument to convert.

        Returns:
            str: A string containing a Rust output declaration for a wrapper function.

        """
        decl = f"{to_safe_snakecase(arg.name)}_out: {self.abi_size_type},"
        if arg.is_list() or arg.as_datatype() == ast.DataType.STRING:
            decl += f"\n{to_safe_snakecase(arg.name)}_out_len:" f" {self.abi_size_type}"

        return decl


@dataclass
class WriteOp:
    target: str
    source: str
    rust_type: str


@dataclass
class AllocOp:
    write_op: WriteOp
    item_type: typing.Union[str, ast.Record, ast.Enumeration]


class ReturnValueWriter:

    write_ops: list[WriteOp]
    alloc_ops: list[AllocOp]

    def __init__(
        self, target: typing.Union[typing.Tuple[str, ast.Record], ast.Function]
    ) -> None:
        if isinstance(target, ast.Function):
            multi_rets = target.is_multi_return()
            (self.write_ops, self.alloc_ops) = self._create_ops(
                target.return_values,
                source_fn=lambda rv: to_safe_snakecase(rv.name) if multi_rets else "",
                source_len_fn=lambda rv: (
                    (to_safe_snakecase(rv.name) if multi_rets else "") + ".len()"
                ),
                target_fn=lambda rv: f"{to_safe_snakecase(rv.name)}_out",
                target_len_fn=lambda rv: f"{to_safe_snakecase(rv.name)}_out_len",
            )
        else:
            argument_name, target_ = target

            def _field_target(field: ast.NamedTypeMixin) -> str:
                size_ops = [f"{to_safe_snakecase(argument_name)}_out"]
                # mypy does not understand that rec is not None here
                # (but does outside the function)
                for rec_field in target_.fields.values():  # type: ignore
                    if rec_field == field:
                        break
                    if (
                        rec_field.is_list()
                        or rec_field.as_datatype() == ast.DataType.STRING
                    ):
                        size_ops.append(
                            f"2 * std::mem::size_of::<{Types.abi_size_type}>()"
                        )
                    elif rec_field.is_record():
                        size_ops.append(
                            to_safe_snakecase(rec_field.type_name()).upper()
                        )
                    else:
                        type_name = Types.abi_types[rec_field.type_name()]
                        size_ops.append(f"std::mem::size_of::<{type_name}>()")
                return " + ".join(size_ops)

            def _field_target_len(field: ast.NamedTypeMixin) -> str:
                return (
                    f"{_field_target(field)}"
                    f" + std::mem::size_of::<{Types.abi_size_type}>()"
                )

            self.write_ops, self.alloc_ops = ReturnValueWriter._create_ops(
                target_.fields,
                source_fn=lambda f: f"{to_safe_snakecase(f.name)}",
                source_len_fn=lambda f: f"{to_safe_snakecase(f.name)}.len()",
                target_fn=_field_target,
                target_len_fn=_field_target_len,
            )

    @staticmethod
    def _create_ops(
        args: typing.Mapping[str, ast.NamedTypeMixin],
        source_fn: typing.Callable[[ast.NamedTypeMixin], str],
        source_len_fn: typing.Callable[[ast.NamedTypeMixin], str],
        target_fn: typing.Callable[[ast.NamedTypeMixin], str],
        target_len_fn: typing.Callable[[ast.NamedTypeMixin], str],
    ) -> typing.Tuple[list[WriteOp], list[AllocOp]]:
        alloc_ops = []
        write_ops = []

        for name, rv in args.items():
            if rv.is_list() or rv.as_datatype() == ast.DataType.STRING:
                list_types = Types.abi_types
                list_types[ast.DataType.STRING] = "u8"
                if rv.is_record():
                    item_type = rv.as_record()
                elif rv.is_enum():
                    item_type = rv.as_enum()
                #                elif rv.as_datatype() == ast.DataType.BOOL:
                #                    item_type = "BEGA"
                else:
                    item_type = list_types[rv.type_name()]

                alloc_ops.append(
                    AllocOp(
                        write_op=WriteOp(
                            target=target_fn(rv),
                            source=source_fn(rv),
                            rust_type=Types.abi_size_type,
                        ),
                        # TODO: booleans and enums
                        item_type=item_type,
                    )
                )

                write_ops.append(
                    WriteOp(
                        target=target_len_fn(rv),
                        source=source_len_fn(rv),
                        rust_type=Types.abi_size_type,
                    )
                )
            elif rv.is_record():
                rec_writer = ReturnValueWriter(target=(rv.name, rv.as_record()))
                write_ops.extend(rec_writer.write_ops)
                alloc_ops.extend(rec_writer.alloc_ops)
            elif rv.is_enum() or rv.as_datatype() == ast.DataType.BOOL:
                write_ops.append(
                    WriteOp(
                        target=target_fn(rv),
                        source=source_fn(rv),
                        rust_type="u8",
                    )
                )
            else:
                write_ops.append(
                    WriteOp(
                        target=target_fn(rv),
                        source=source_fn(rv),
                        rust_type=Types.abi_types[rv.type_name()],
                    )
                )

        return (write_ops, alloc_ops)

    @staticmethod
    def _data_source_name(data_source_name: str, op: WriteOp) -> str:
        return f"{data_source_name}.{op.source}" if op.source else data_source_name

    @staticmethod
    def _write_op(data_source_name: str, op: WriteOp) -> str:
        return (
            f"*(unsafe {{ mem_base.add({op.target} as usize)"
            f" as *mut {op.rust_type}}}) = "
            f"{ReturnValueWriter._data_source_name(data_source_name, op)} as {op.rust_type}"
        )

    def _write_destructure(self, data_source_name: str) -> str:

        write_op_lajns = map(
            lambda write_op: (f"{self._write_op(data_source_name, write_op)};"),
            self.write_ops,
        )

        alloc_lines = map(
            lambda alloc_op: f"""let allocs = [(
    {self._data_source_name(data_source_name, alloc_op.write_op)}.as_ptr(),
    {self._data_source_name(data_source_name, alloc_op.write_op)}.len(),
    *(unsafe {{ mem_base.add({alloc_op.write_op.target} as\
 usize) as *mut {alloc_op.write_op.rust_type}}})
)];""",
            self.alloc_ops,
        )

        return "\n".join(
            itertools.chain(
                [
                    "// destructure values into write-ops and alloc-ops",
                    "// write ops",
                ],
                write_op_lajns,
                alloc_lines,
            )
        )

    def _write_alloc(self) -> str:
        """Write stuff version 2."""

        def alloc_line(alloc_input: typing.Tuple[int, AllocOp]) -> str:
            index, alloc_op = alloc_input
            return f"""unsafe {{
    let offset = try_or_errmsg!(caller, allocate(
        &mut caller,
        std::mem::size_of::<{alloc_op.item_type}>() * allocs[{index}].1
    ));
    {_copy_data(index, alloc_op)}
    allocs[{index}].2 = offset;
}};"""

        def _copy_data(index: int, alloc_op: AllocOp) -> str:
            if isinstance(alloc_op.item_type, ast.Record):
                item_conversion = ReturnValueWriter(
                    target=(alloc_op.write_op.target, alloc_op.item_type)
                ).generate(data_source_name="item", indent=4)
                return f"""
    for item in std::slice::from_raw_parts(allocs[{index}].0, allocs[{index}].1).iter() {{
        {item_conversion}
    }}
"""

            return f"""(mem_base.add(offset as usize) as *mut {alloc_op.item_type})
        .copy_from_nonoverlapping(
            allocs[{index}].0,
            allocs[{index}].1,
        );"""

        alloc_lajns = map(
            alloc_line,
            enumerate(self.alloc_ops),
        )
        return "\n".join(itertools.chain(["// allocate needed"], alloc_lajns))

    def generate(self, data_source_name: str, indent: int = 0) -> str:
        return "\n" + textwrap.indent(
            "\n".join(
                [
                    self._write_destructure(data_source_name=data_source_name),
                    self._write_alloc(),
                ]
            ),
            prefix=" " * indent,
        )


def attach_return_value_writers(functions: dict[str, ast.Function]) -> None:

    for fun in functions.values():
        setattr(fun, "return_value_writer", ReturnValueWriter(fun))


class RustWasmtime(Target):
    """
    Rust Wasmtime target

    Generates host side bindings for the API to use with the wasmtime library.
    """

    name: str = "rust-wasmtime"

    def generate(
        self, module: ast.Module, options: argparse.Namespace
    ) -> typing.Iterator[str]:
        attach_return_value_writers(module.functions)
        environment = jinja2.Environment(
            loader=jinja2.PackageLoader(
                "turbo_islc.targets.rust_wasmtime", "templates"
            ),
            autoescape=jinja2.select_autoescape(),
            trim_blocks=True,
            lstrip_blocks=True,
        )

        jinja_common.add_common_filters(environment)
        environment.filters["snakecase"] = to_safe_snakecase

        lifetime = Lifetime()
        return environment.get_template("module.jinja2.rs").generate(
            module_name=module.name,
            functions=module.functions,
            records=module.records,
            enums=module.enums,
            lifetime=lifetime,
            types=Types(lifetime=lifetime),
        )

    @staticmethod
    def argparser() -> typing.Optional[argparse.ArgumentParser]:
        """Get argparser."""
        return None
