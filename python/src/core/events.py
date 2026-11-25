"""Events the core reports to the screen. The core never builds text: a notice is a code and its arguments, and the
screen turns it into a sentence with the locale. No import from the screen or the locale here."""

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Message:
    """A chat line from a peer or from this PC: sender name, text, whether it is this PC's own line."""
    sender: str
    text: str
    mine: bool = False
    plain: bool = False  # a plain (unverified, no key) sender: the screen labels it "(plain text)"


@dataclass(frozen=True)
class Notice:
    """A notice by code (the locale key) and its arguments. A notice has no sender."""
    code: str
    args: dict = field(default_factory=dict)


@dataclass(frozen=True)
class Line:
    """Text the screen side already made (a command reply, a status text). Core modules do not send it."""
    text: str
    mine: bool = False


@dataclass(frozen=True)
class PeerRow:
    """One /peers line. state: "verified", "plain", "unverified" (code = the short code), "pending" or "" (no text)."""
    prefix: str
    name: str
    ip: str
    port: int
    state: str
    code: str = None


@dataclass(frozen=True)
class Status:
    """The relay's state for the status row: a short text, empty when there is nothing to say."""
    text: str
