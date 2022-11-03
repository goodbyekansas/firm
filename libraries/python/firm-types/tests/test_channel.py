""" Test the Stream class and associated functions """
import typing

import pytest

from firm_types import channel
from firm_types.types import execution


def test_channel() -> None:
    """test converting to a channel from a value"""
    chan = channel.channel(1)
    assert chan.WhichOneof("value") == "integers"
    assert chan.integers == execution.Integers(values=[1])

    chan = channel.channel("I am a string")
    assert chan.WhichOneof("value") == "strings"
    assert chan.strings == execution.Strings(values=["I am a string"])

    chan = channel.channel(False)
    assert chan.WhichOneof("value") == "booleans"
    assert chan.booleans == execution.Booleans(values=[False])

    chan = channel.channel(2.00000000000000000000000003)
    assert chan.WhichOneof("value") == "floats"
    assert chan.floats == execution.Floats(values=[2.00000000000000000000000003])

    chan = channel.channel(b"hej")
    assert chan.WhichOneof("value") == "bytes"
    assert chan.bytes == execution.Bytes(values=b"hej")

    chan = channel.channel([2])
    assert chan.WhichOneof("value") == "integers"
    assert chan.integers == execution.Integers(values=[2])

    chan = channel.channel(["I am first string", "I am second string"])
    assert chan.WhichOneof("value") == "strings"
    assert chan.strings == execution.Strings(
        values=["I am first string", "I am second string"]
    )

    chan = channel.channel([2, 3])
    assert chan.WhichOneof("value") == "integers"
    assert chan.integers == execution.Integers(values=[2, 3])

    chan = channel.channel([1.23, 2.34])
    assert chan.WhichOneof("value") == "floats"
    assert chan.floats == execution.Floats(values=[1.23, 2.34])

    chan = channel.channel([True, False])
    assert chan.WhichOneof("value") == "booleans"
    assert chan.booleans == execution.Booleans(values=[True, False])

    empty_list: typing.List[str] = []
    chan = channel.channel(empty_list)

    with pytest.raises(channel.ChannelConversionError):
        channel.channel([1, 2.2])


def test_value() -> None:
    """Test converting from channels to regular stuff"""
    chan = channel.channel(1)
    assert channel.value(chan) == [1]
    assert channel.value(chan, as_type=int) == 1
    assert channel.value(chan, as_type=typing.List[int]) == [1]
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=typing.List[bool])
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=float)

    chan = channel.channel(1.230000000001)
    assert typing.cast(typing.List[float], channel.value(chan)) == [1.230000000001]
    assert typing.cast(float, channel.value(chan, as_type=float)) == 1.230000000001
    assert typing.cast(
        typing.List[float],
        channel.value(chan, as_type=typing.List[float]),
    ) == [1.230000000001]

    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=typing.List[bool])
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=int)

    chan = channel.channel("ett")
    assert channel.value(chan) == ["ett"]
    assert channel.value(chan, as_type=str) == "ett"
    assert channel.value(chan, as_type=typing.List[str]) == ["ett"]
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=typing.List[bool])
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=float)

    chan = channel.channel(False)
    assert channel.value(chan) == [False]
    assert channel.value(chan, as_type=bool) is False
    assert channel.value(chan, as_type=typing.List[bool]) == [False]
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=typing.List[str])
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=float)

    chan = channel.channel(b"bytes")
    assert channel.value(chan) == b"bytes"
    assert channel.value(chan, as_type=bytes) == b"bytes"
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=typing.List[bool])
    with pytest.raises(channel.ChannelConversionError):
        channel.value(chan, as_type=float)
