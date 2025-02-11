from dataclasses import dataclass
from typing import List, Optional


@dataclass
class DisplayedItemRef:
    """Represents a reference to a displayed item."""
    id: str


@dataclass
class ItemInfo:
    """Represents detailed information about a displayed item."""
    id: DisplayedItemRef
    name: str
    color: Optional[str] = None


@dataclass
class WcpCSMessage:
    """Represents a message from the client to the server."""
    type: str
    version: Optional[str] = None
    commands: Optional[List[str]] = None
    command: Optional['WcpCommand'] = None


@dataclass
class WcpCommand:
    """Represents a command sent by the client."""
    type: str
    ids: Optional[List[DisplayedItemRef]] = None
    names: Optional[List[str]] = None
    scope: Optional[str] = None
    timestamp: Optional[int] = None
    color: Optional[str] = None
    source: Optional[str] = None
    viewport_idx: Optional[int] = None


@dataclass
class WcpSCMessage:
    """Represents a message from the server to the client."""
    type: str
    version: Optional[str] = None
    commands: Optional[List[str]] = None
    response: Optional['WcpResponse'] = None
    error: Optional[str] = None
    arguments: Optional[List[str]] = None
    message: Optional[str] = None
    event: Optional['WcpEvent'] = None


@dataclass
class WcpResponse:
    """Represents a response from the server."""
    type: str
    item_list: Optional[List[str]] = None
    item_info: Optional[List[ItemInfo]] = None
    added_items: Optional[List[DisplayedItemRef]] = None


@dataclass
class WcpEvent:
    """Represents an event sent by the server."""
    type: str
