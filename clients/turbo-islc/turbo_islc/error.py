"""Error types for TurboISL."""

import io
import typing


class TurboException(Exception):
    """Exception for Turbo ISLC mishaps.

    Attributes:
        message (str): The error message.
        file_name (str): The TISL file being parsed.
    """

    message: str
    file_name: str

    def __init__(self, message: str, filename: str = "<text>"):
        """Init for turbo exception.

        Args:
            message (str): The error message.
            filename (str): File being parsed.

        """
        self.message = message
        if isinstance(filename, io.TextIOWrapper):
            self.file_name = filename.name
        else:
            self.file_name = filename
        super().__init__()

    def __str__(self) -> str:
        """__str__."""
        return f"{self.file_name}: Internal Error: {self.message}"


class TurboSyntaxError(TurboException):
    """Error when parsing the syntax.

    Attributes:
        lineno (int): The line number the error was detected at.
        column (int): The column the error was detected at.
        line (Optional[str]): The line with the error.
    """

    lineno: int
    column: int
    line: typing.Optional[str]

    # pylint: disable=too-many-arguments
    def __init__(
        self,
        message: str,
        lineno: int,
        column: int,
        filename: str = "<text>",
        line: typing.Optional[str] = None,
    ):
        """Init for turbo exception.

        Args:
            message (str): The error message.
            filename (str): File being parsed.
            lineno (int): Line number of the error.
            column (int): Column of the error.
            line (str): The faulting line.

        """
        self.lineno = lineno
        self.column = column
        self.line = line
        super().__init__(message=message, filename=filename)

    def __str__(self) -> str:
        pointer = ""
        if self.line:
            pointer = f"""
    {self.line}
    {' '*self.column}^"""

        return (
            f"{self.file_name}:{self.lineno}:{self.column}: "
            f"Error: {self.message}{pointer}"
        )
