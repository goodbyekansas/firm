"""Base types for implementing targets."""
import argparse
import typing
from abc import ABC, abstractmethod

from turbo_islc.ast import Module, SyntaxNode


class Target(ABC):
    """Base class for all TurboISL targets.

    A target describe how code gets generated and calling the `generate` function on a
    target produces a string iterator from a module syntax tree node.

    Attributes:
        name (str): Name of the target (used as a command line argument)

    """

    name: str

    @abstractmethod
    def generate(
        self, module: Module, options: argparse.Namespace
    ) -> typing.Iterator[str]:
        """Generate code a module with the given options.

        Note that if the TISL code contains multiple modules, this function will be
        called multiple times, once per module.

        Args:
            module (Module): Module to generate code for.
            options (argparse.Namespace): Command line options specific to this target.

        Returns:
            Iterator[str]: An iterator over generated strings for `module`.

        """

    @staticmethod
    @abstractmethod
    def argparser() -> typing.Optional[argparse.ArgumentParser]:
        """Argument parser for this target.

        This argument parser will get merged in with the main argument parser for the
        Turbo ISL Compiler.

        Returns:
            argparse.ArgumentParser: A new argument parser capable of parsing
                                     options for this target.

        """
