"""Turbo-ISL Compiler syntax tree types."""
from __future__ import annotations

import typing
from abc import ABC
from enum import Enum, auto

import pyparsing as pp

from turbo_islc.error import TurboSyntaxError


class SyntaxNode(ABC):  # pylint: disable=too-few-public-methods
    """Abstract syntax node.

    Common base for all nodes in a syntax tree.

    Attributes:
        parent_module (Module): All syntax nodes (except modules) have a module as a
                                parent which can be accessed through this attribute.
    """

    parent_module: typing.Optional[Module]

    def __init__(self, module: typing.Optional[Module], *_args: list):
        """Perform common initialization.

        Args:
            module (Optional[Module]): Optional parent module for this syntax node.
        """
        self.parent_module = module

    def node_type(self) -> str:
        """String representation of the node type."""
        return self.__class__.__name__.lower()


def turbo_keyword(keyword: str) -> typing.Callable:
    """Decorator to associate a turbo keyword with a syntax node.

    This is done so that the parsing code knows which type of syntax node to create.

    Args:
        keyword (str): The keyword.
    """

    def _turbo_keyword(node: type) -> type:
        setattr(node, "turbo_keyword", keyword)
        return node

    return _turbo_keyword


@turbo_keyword("mod")
class Module(SyntaxNode):  # pylint: disable=too-few-public-methods
    """Module syntax node.

    All other definitions live inside a module, which provides a common name.

    Attributes:
        name (str): Name of the module.
        doc_string (str): Documentation for the module.
        members (list[SyntaxNode]): All module members.
        functions (dict[str, Function]): Function members of the module.
        records (dict[str, Record]): Record members of the module.
        enums (dict[str, Enumeration]): Enum members of the module.

    """

    name: str
    doc_string: str
    members: list[SyntaxNode]
    functions: dict[str, Function]
    records: dict[str, Record]
    enums: dict[str, Enumeration]

    def __init__(
        self, module: typing.Optional[Module], name: str, doc_string: str, members: list
    ):
        """Initialize a new Module.

        Args:
            module (Optional[Module]): Optional parent module for this module.
            name (str): Module name.
            doc_string (str): Module documentation.
            members: (list): Un-parsed list of members.

        """
        self.name = name
        self.doc_string = doc_string
        self.members = list(map(lambda m: parse(m, self), members))
        self.functions = {
            function.name: function
            for function in self.members
            if isinstance(function, Function)
        }
        self.records = {
            record.name: record for record in self.members if isinstance(record, Record)
        }
        self.enums = {
            enum.name: enum for enum in self.members if isinstance(enum, Enumeration)
        }
        super().__init__(module)

    def is_root_module(self) -> bool:
        """Determine if this is a root module.

        A root module has no parent, i.e. is not nested inside another module.

        Returns:
            bool: True if this module is not nested inside another module.
        """
        return self.parent_module is None

    def function(
        self, name: str, default: typing.Optional[Function] = None
    ) -> typing.Optional[Function]:
        """Get a function by name in this module.

        Args:
            name (str): Name of the function.
            default (Optional[Function]): If a function with `name` does not exist,
                                          use this instead.

        Returns:
            Optional[Function]: A function named `name` if it exists,
                                otherwise `default` and lastly, None.

        """
        return self.functions.get(name, default)

    def record(
        self, name: str, default: typing.Optional[Record] = None
    ) -> typing.Optional[Record]:
        """Get a record by name in this module.

        Args:
            name (str): Name of the record.
            default (Optional[Record]): If a record with `name` does not exist,
                                        use this instead.

        Returns:
            Optional[Record]: A record named `name` if it exists,
                              otherwise `default` and lastly, None.

        """
        return self.records.get(name, default)

    def enum(
        self, name: str, default: typing.Optional[Enumeration] = None
    ) -> typing.Optional[Enumeration]:
        """Get a enum by name in this module.

        Args:
            name (str): Name of the enum.
            default (Optional[Enumeration]): If an enum with `name` does not exist,
                                             use this instead.

        Returns:
            Optional[Enumeration]: An enum named `name` if it exists,
                                   otherwise `default` and lastly, None.

        """
        return self.enums.get(name, default)

    def parent(self) -> typing.Optional[Module]:
        """Get the parent module, if it exists.

        Returns:
            Optional[Module]: The parent module or None.

        """
        return self.parent_module


class DataType(Enum):
    """Enum for supported data types."""

    INT = "int"
    FLOAT = "float"
    STRING = "string"
    BOOL = "bool"
    BYTES = "bytes"

    @staticmethod
    def parse(expr: str) -> DataType:
        """Create a DataType from from its name.

        Args:
            expr (str): String representation of an enum variant.

        Returns:
            DataType: An enum variant, if matching.

        """
        return DataType[expr.upper()]

    def __str__(self) -> str:
        return self.value


class Modifier(Enum):
    """Modifer for data types.

    A modifier is a hint for the code generation. It does not change the type.
    """

    LIST = auto()
    REF = auto()

    @staticmethod
    def parse(expr: str) -> Modifier:
        """Create a Modifier from from its name.

        Args:
            expr (str): String representation of an enum variant.

        Returns:
            Modifier: An enum variant, if matching.

        """
        return Modifier[expr.upper().strip()]


class NamedTypeMixin(ABC):  # pylint: disable=too-few-public-methods
    """Base class for a (name, type) tuple.

    Attributes:
        name (str): The name of the named type.
        data_type (typing.Union[DataType, str]): The data type this represents.
        modifiers (list[Modifier]): All modifier hints for this type.
        parent (SyntaxNode): The parent (function, record etc) this type belongs to.

    """

    name: str
    data_type: typing.Union[DataType, str]
    modifiers: list[Modifier]
    parent: SyntaxNode
    line: int
    column: int

    def __init__(
        self,
        module: Module,
        parent: SyntaxNode,
        name: str,
        data_type: dict[str, typing.Any],
    ):
        """Initialize the NamedType.

        Args:
            module (Module): The module this type's parent belongs to.
            parent (SyntaxNode): Parent object for this NamedType.
            name (str): Name of the NamedType.
            data_type (Union[str, ParseResults]): Data type this represents.

        """
        super().__init__(module)  # type: ignore  # should be fixed in mypy 0.930
        self.name = name
        self.parent = parent
        self.line = data_type["line"]
        self.column = data_type["column"]
        self.modifiers = []
        if data_type["type"] == "builtin":
            try:
                self.data_type = DataType.parse(data_type["name"])
            except KeyError as err:
                raise TurboSyntaxError(
                    message=f"Unexpected data type {err.args[0]}",
                    lineno=data_type["line"],
                    column=data_type["column"],
                ) from err
        elif data_type["type"] == "record-or-enum":
            self.data_type = data_type["name"]

        if mods := data_type.get("modifiers"):
            try:
                self.modifiers = list(map(Modifier.parse, mods))
            except KeyError as err:
                raise TurboSyntaxError(
                    message=f"Unexpected modifier {err.args[0]}",
                    lineno=data_type["line"],
                    column=data_type["column"],
                ) from err
        if self.data_type == DataType.BYTES and not self.is_list():
            self.modifiers.append(Modifier.LIST)

    def is_reference(self) -> bool:
        """Determine if NamedType is a reference

        Returns:
            bool: True if it has the reference modifier hint, else False

        """
        return Modifier.REF in self.modifiers

    def is_list(self) -> bool:
        """Determine if NamedType is a list.

        Returns:
            bool: True if it has the list modifier hint, else False

        """
        return Modifier.LIST in self.modifiers

    def is_record(self) -> bool:
        """Determine if this NamedType is a Record.

        Returns:
            bool: True if this is a record else False

        """
        return isinstance(
            self.data_type, str
        ) and self.parent_module.record(  # type: ignore
            self.data_type
        )

    def is_enum(self) -> bool:
        """Determine if this NamedType is an Enumeration.

        Returns:
            bool: True if this is an enumeration else False

        """
        return isinstance(
            self.data_type, str
        ) and self.parent_module.enum(  # type: ignore
            self.data_type
        )

    def is_simple_type(self) -> bool:
        """Determine if this NamedType is a DataType.

        Returns:
            bool: True if this type is one of the simple types.

        """
        return isinstance(self.data_type, DataType)

    def type_name(self) -> str:
        """Get the name of the type.

        Returns:
            str: the name of the type.

        """
        return str(self.data_type)

    def as_record(self) -> typing.Optional[Record]:
        """Get the record if it is a record.

        Returns:
            Optional[Record]: A record of this NamedType, if it is one else None.

        """
        if isinstance(self.data_type, str):
            rec = self.parent_module.record(self.data_type)  # type: ignore
            if not rec:
                raise TurboSyntaxError(
                    message=f"Undefined data type '{self.data_type}'",
                    lineno=self.line,
                    column=self.column,
                )
            return rec
        return None

    def as_enum(self) -> typing.Optional[Record]:
        """Get the enum if it is a enum.

        Returns:
            Optional[Enum]: An enum of this NamedType, if it is one else None.

        """
        if isinstance(self.data_type, str):
            enu = self.parent_module.enum(self.data_type)  # type: ignore
            if not enu:
                raise TurboSyntaxError(
                    message=f"Undefined data type '{self.data_type}'",
                    lineno=self.line,
                    column=self.column,
                )
            return enu
        return None

    def as_datatype(self) -> typing.Optional[DataType]:
        """Get the datatype.

        Returns:
            Optional[DataType]: Get the NamedType as its DataType
                                if it is a simple type else None.

        """
        if isinstance(self.data_type, DataType):
            return self.data_type
        return None


class FunctionArgument(
    NamedTypeMixin, SyntaxNode
):  # pylint: disable=too-few-public-methods
    """Function argument."""


class FunctionReturnValue(
    NamedTypeMixin, SyntaxNode
):  # pylint: disable=too-few-public-methods
    """Function return value."""


class Field(NamedTypeMixin, SyntaxNode):  # pylint: disable=too-few-public-methods
    """Field for Record"""


@turbo_keyword("fun")
class Function(SyntaxNode):  # pylint: disable=too-few-public-methods
    """Function syntax node.

    Attributes:
        name (str): The name of the Function.
        doc_string (str): The documentation of the function.
        arguments (dict[str, FunctionArgument]): Arguments this function takes.
        return_values (dict[str, FunctionReturnValue]): Return values of this function.

    """

    name: str
    doc_string: str
    arguments: dict[str, FunctionArgument]
    return_values: dict[str, FunctionReturnValue]

    # pylint: disable=too-many-arguments
    def __init__(
        self,
        module: Module,
        name: str,
        doc_string: str,
        args: list,
        return_values: list,
    ):
        """Initialize a Function.

        Args:
            module (Module): The parent module of the function.
            name (str): The name of the function.
            doc_string (str): Documentation of the function.
            args (list): Arguments the function takes.
            return_values (list): Return values for the function.

        """
        self.module = module
        self.name = name
        self.doc_string = doc_string

        self.arguments = {
            sym[0]: FunctionArgument(module, self, *sym)
            for sym in zip(args[::2], args[1::2])
        }
        self.return_values = {
            sym[0]: FunctionReturnValue(module, self, *sym)
            for sym in zip(return_values[::2], return_values[1::2])
        }
        super().__init__(module)

    def is_multi_return(self) -> bool:
        """Determine if function is multi return.

        Returns:
            bool: True if the function returns multiple values, else False.

        """
        return len(self.return_values) > 1

    def has_return_values(self) -> bool:
        """Determine if function has return values.

        Returns:
            bool: True if the function returns anything, else False.

        """
        return len(self.return_values) > 0

    def num_returns(self) -> int:
        """Return the number of values the function returns.

        Returns:
            int: The number of return values.

        """
        return len(self.return_values)

    def num_arguments(self) -> int:
        """Tells the number of arguments the function takes.

        Returns:
            int: The number of input arguments for the function.

        """
        return len(self.arguments)

    def get_first_return_value(self) -> typing.Optional[FunctionReturnValue]:
        """Get the first return value.

        Returns:
            optional[FunctionReturnValue]: Get the first return value, or None.

        """
        return next(iter(self.return_values.values()), None)


@turbo_keyword("enu")
class Enumeration(SyntaxNode):
    """Enum syntax node.

    Attributes:
        module: The module this enum belongs to.
        name: The name of the enum.
        doc_string: The documentation of this enum.
        variants: All possible variants for this enum.

    """

    module: Module
    name: str
    doc_string: str
    variants: list[str]

    def __init__(self, module: Module, name: str, doc_string: str, variants: list):
        """Initialize the Enum

        Args:
            module (Module): the parent module of this enum.
            name (str): The name of the enum.
            doc_string (str): The documentation of this enum.
            variants (list): The fields this enum has.

        """

        self.module = module
        self.name = name
        self.doc_string = doc_string
        self.variants = list(map(str, variants))
        super().__init__(module)

    def __iter__(self) -> typing.Iterator[str]:
        return iter(self.variants)

    def __contains__(self, key: str) -> bool:
        return key in self.variants


@turbo_keyword("rec")
class Record(SyntaxNode):  # pylint: disable=too-few-public-methods
    """Record syntax node.

    Attributes:
        module (Module): The module this record belongs to.
        name (str): The name of the Record.
        doc_string (str): The documentation of the record.
        fields (dict[str, Field]): All record fields, indexed by their name.

    """

    module: Module
    name: str
    doc_string: str
    fields: dict[str, Field]

    def __init__(self, module: Module, name: str, doc_string: str, fields: list):
        """Initialize the Record

        Args:
            module (Module): the parent module of this Record.
            name (str): The name of the record.
            doc_string (str): The documentation of this record.
            fields (list): The fields this record has.

        """
        self.module = module
        self.name = name
        self.doc_string = doc_string

        self.fields = {
            sym[0]: Field(module, self, *sym) for sym in zip(fields[::2], fields[1::2])
        }
        super().__init__(module)

    def __getitem__(self, key: str) -> Field:
        return self.fields[key]

    def __iter__(self) -> typing.Iterator[str]:
        return iter(self.fields)

    def iter_fields(self) -> typing.ItemsView[str, Field]:
        """Iterate over the fields.

        Yields:
            (str, Field): The next field in the record.

        """
        return self.fields.items()


PARSERS = {
    getattr(child, "turbo_keyword"): child
    for child in SyntaxNode.__subclasses__()
    if hasattr(child, "turbo_keyword")
}


def lex(turbo_isl: str) -> pp.ParseResults:  # pylint: disable=too-many-locals
    """Read the turbo ISL and parse it to pyparsing tokens.

    Args:
        turbo_isl (str): The string to read.

    Returns:
        ParseResults: The pyparsing representation of the syntax tree.

    """

    begin = pp.Suppress(pp.Word("([{", exact=1).setName("Opening brace"))
    end = pp.Suppress(pp.Word(")]}", exact=1).setName("Closing brace"))

    def _parse_builtin_type(
        orig: str, loc: int, toks: pp.ParseResults
    ) -> pp.ParseResults:
        return pp.ParseResults(
            toklist=[
                {
                    "name": toks[0],
                    "type": "builtin",
                    "line": pp.lineno(loc, orig),
                    "column": pp.col(loc, orig),
                }
            ]
        )

    def _parse_record_or_enum(
        orig: str, loc: int, toks: pp.ParseResults
    ) -> pp.ParseResults:
        return pp.ParseResults(
            toklist=[
                {
                    "name": toks[0],
                    "type": "record-or-enum",
                    "line": pp.lineno(loc, orig),
                    "column": pp.col(loc, orig),
                }
            ]
        )

    def _parse_modifiers(toks: pp.ParseResults) -> pp.ParseResults:
        return pp.ParseResults(
            toklist=[dict(toks[0][-1]) | {"modifiers": toks[0][:-1]}]
        )

    def _parse_id(orig: str, loc: int, keyword: pp.ParseResults) -> None:
        raise TurboSyntaxError(
            message=(
                f'Disallowed character in name: "{keyword[0]}"'
                ", allowed are: a-z, A-Z and -"
            ),
            lineno=pp.lineno(loc, orig),
            column=pp.col(loc, orig),
            line=pp.line(loc, orig),
        )

    identifier = (
        (
            pp.Optional(pp.Suppress(":"))
            + pp.Word(pp.alphas, pp.alphanums + "-")
            + pp.FollowedBy(pp.White() | end)
        )
        | pp.Regex(r"[^\s\(\)\[\]\{\}]+").setParseAction(_parse_id)
    ).setResultsName("identifier")

    actual_data_type = (
        pp.Keyword("string").setResultsName("string")
        | pp.Keyword("int").setResultsName("int")
        | pp.Keyword("float").setResultsName("float")
        | pp.Keyword("bool").setResultsName("bool")
        | pp.Keyword("bytes").setResultsName("bytes")
    ).setParseAction(_parse_builtin_type) | identifier.setResultsName(
        "record-type"
    ).setParseAction(
        _parse_record_or_enum
    )

    data_type = (
        actual_data_type
        | begin
        + pp.Group((pp.Keyword("list") | pp.Keyword("ref"))[1, ...] + actual_data_type)
        .setParseAction(_parse_modifiers)
        .setResultsName("modifiers")
        + end
    )
    docstring = pp.QuotedString(quoteChar='"', multiline=True)

    arg_list = pp.Group(
        begin
        + ((identifier.setResultsName("name") - data_type).setName("Data type")[...])
        .setName("ArgumentList")
        .setResultsName("argument")
        + end
    )

    function = (
        begin
        + pp.Group(
            pp.Keyword("fun")
            - identifier.setResultsName("name")
            - pp.Optional(docstring, default="")
            .setResultsName("docstring")
            .setName("Documentation string")
            - arg_list.setName("Arguments").setResultsName("arguments")
            - arg_list.setName("Return Values").setResultsName("return-values")
        )
        .setName("Function")
        .setResultsName("function")
        + end
    )

    enum = (
        begin
        + pp.Group(
            pp.Keyword("enu")
            - identifier.setResultsName("name")
            - pp.Optional(docstring, default="")
            .setResultsName("docstring")
            .setName("Documentation string")
            - pp.Group(
                begin
                + ((identifier.setResultsName("variant"))[...])
                .setName("Variants")
                .setResultsName("variants")
                + end
            )
        )
        .setName("Enumeration")
        .setResultsName("enum")
        + end
    )

    record = (
        begin
        + pp.Group(
            pp.Keyword("rec")
            - identifier.setResultsName("name")
            - pp.Optional(docstring, default="")
            .setResultsName("docstring")
            .setName("Documentation string")
            + arg_list.setName("Fields").setResultsName("fields")
        )
        .setName("Record")
        .setResultsName("record")
        + end
    )

    def _unknown_keyword(orig: str, loc: int, keyword: pp.ParseResults) -> None:
        raise TurboSyntaxError(
            message=f'Unexpected keyword: "{keyword[0]}"',
            lineno=pp.lineno(loc, orig),
            column=pp.col(loc, orig),
            line=pp.line(loc, orig),
        )

    module = pp.Forward()
    module <<= (
        begin
        + pp.Group(
            pp.Keyword("mod")
            - identifier.setResultsName("name")
            - pp.Optional(docstring, default="").setName("docstring")
            - pp.Group(
                (
                    function
                    | record
                    | enum
                    | module
                    | (begin + identifier.setParseAction(_unknown_keyword))
                )[1, ...]
                | pp.Empty()
            ).setResultsName("members")
        )
        .setName("Module")
        .setResultsName("module")
        + end
    )
    return module.ignore(";" + pp.restOfLine)[...].parseString(turbo_isl, parseAll=True)


def parse(root: pp.ParseResults, module: typing.Optional[Module] = None) -> SyntaxNode:
    """Generate a single syntax node from a symbol list.

    Args:
        root (ParseResults): A parsed TISL to turn into a syntax tree.
        module (Optional[Module]): A module to put the syntax tree under.

    Returns:
        SyntaxNode: The root of the generated syntax tree.

    """
    return PARSERS[root[0]](module, *root[1:])
