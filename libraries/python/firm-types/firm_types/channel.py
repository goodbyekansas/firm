""" Convenience functions dealing with channels """
import typing

from firm_types.types import execution

ChannelTypes = typing.Union[
    str,
    float,
    int,
    bool,
    bytes,
    typing.List[str],
    typing.List[float],
    typing.List[int],
    typing.List[bool],
]


class ChannelConversionError(BaseException):
    """Errors for converting to and from channels"""


def channel(subject: ChannelTypes) -> execution.Channel:
    """Convert an object to a channel"""

    if subject == []:
        return execution.Channel()
    if isinstance(subject, bytes):
        return execution.Channel(bytes=execution.Bytes(values=subject))

    subject_list = subject if isinstance(subject, list) else [subject]
    try:
        if isinstance(subject_list[0], str):
            return execution.Channel(
                strings=execution.Strings(
                    values=typing.cast(typing.List[str], subject_list)
                )
            )
        if isinstance(subject_list[0], float):
            return execution.Channel(
                floats=execution.Floats(
                    values=typing.cast(typing.List[float], subject_list)
                )
            )
        if isinstance(subject_list[0], bool):
            return execution.Channel(booleans=execution.Booleans(values=subject_list))
        if isinstance(subject_list[0], int):
            return execution.Channel(integers=execution.Integers(values=subject_list))
    except TypeError as error:
        ChannelConversionError(error)

    raise ChannelConversionError(
        f"could not convert from {subject.__class__} to channel"
    )


def value(
    from_channel: execution.Channel, as_type: typing.Optional[type] = None
) -> ChannelTypes:
    """Convert from a channel to a python type"""

    def get_exactly_one(values: typing.List) -> ChannelTypes:
        if len(values) == 1:
            return values[0]
        if len(values) > 1:
            raise ChannelConversionError(
                "Channel has more than one value, asked for exactly one"
            )
        raise ChannelConversionError("No values in channel, asked for exactly one")

    which_one = from_channel.WhichOneof("value")
    if not which_one:
        raise ChannelConversionError(
            f"Could not get type to convert from, got {which_one}"
        )
    channel_value = getattr(from_channel, which_one).values
    if as_type is None:
        return channel_value

    type_map = {
        "strings": (str, typing.List[str]),
        "floats": (float, typing.List[float]),
        "integers": (int, typing.List[int]),
        "booleans": (bool, typing.List[bool]),
        "bytes": (bytes,),
    }
    if as_type not in type_map.get(which_one, (None,)):
        raise ChannelConversionError(
            f"Could not convert from {which_one} to {str(as_type)}"
        )

    return (
        get_exactly_one(channel_value)
        if as_type in (str, float, int, bool)
        else channel_value
    )
