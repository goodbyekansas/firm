"""Common helper filters for jinja2"""
import itertools
import typing

import jinja2


def to_camelcase(kebab: str) -> str:
    """convert str from kebab case to camel case"""
    return "".join(map(str.title, kebab.split("-")))


def to_snakecase(kebab: str) -> str:
    """convert str from kebab case to snake case"""
    return kebab.replace("-", "_")


def chain(
    first: typing.Iterator[typing.Any], second: typing.Iterator[typing.Any]
) -> typing.Iterator[typing.Any]:
    """Chain two iterators together, creating a new iterator.

    Args:
        first (Iterator): Iterator to start from.
        secord (Iterator): Iterator to add to the first iterator.

    Returns:
        Iterator: An iterator that iterates "first" then "second" as if they were one.

    """
    return itertools.chain(first, second)


def add_common_filters(environment: jinja2.Environment) -> None:
    """Add common useful filters to the jinja environment.

    Args:
        environment (jinja2.Environment): The environment to add filters to.

    """

    environment.filters["camelcase"] = to_camelcase
    environment.filters["snakecase"] = to_snakecase
    environment.filters["chain"] = chain
