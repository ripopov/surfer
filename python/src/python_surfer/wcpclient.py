from dataclasses import dataclass, field
from typing import List, Optional, Dict, Any


@dataclass
class DisplayedItemRef:
    """Represents a reference to a displayed item."""
    id: str


@dataclass
class ItemInfo:
    """Represents detailed information about a displayed item."""
    id: str
    name: str
    color: Optional[str] = None


@dataclass
class WcpCommand:
    """Represents a command sent from the client to the server."""
    command_type: str
    args: Dict[str, Any] = field(default_factory=dict)


@dataclass
class WcpResponse:
    """Represents a response sent from the server to the client."""
    response_type: str
    data: Optional[Any] = None


@dataclass
class WcpEvent:
    """Represents an event sent from the server to the client."""
    event_type: str
    data: Optional[Any] = None


@dataclass
class WcpCSMessage:
    """Represents a message sent from the client to the server."""
    message_type: str
    version: Optional[str] = None
    commands: Optional[List[str]] = None
    command: Optional[WcpCommand] = None


@dataclass
class WcpSCMessage:
    """Represents a message sent from the server to the client."""
    message_type: str
    version: Optional[str] = None
    commands: Optional[List[str]] = None
    response: Optional[WcpResponse] = None
    error: Optional[Dict[str, Any]] = None
    event: Optional[WcpEvent] = None


class WcpClient:
    """A client for interacting with the WCP server."""

    def __init__(self, server_url: str):
        """
        Initialize the WCP client.

        :param server_url: The URL of the WCP server.
        """
        self.server_url = server_url

    def send_message(self, message: WcpCSMessage) -> WcpSCMessage:
        """
        Send a message to the WCP server and receive a response.

        :param message: The message to send.
        :return: The response from the server.
        """
        # Simulate sending a message and receiving a response
        response = self._simulate_server_response(message)
        return response

    def _simulate_server_response(self, message: WcpCSMessage) -> WcpSCMessage:
        """
        Simulate a server response for testing purposes.

        :param message: The message sent by the client.
        :return: A simulated server response.
        """
        if message.message_type == "greeting":
            return WcpSCMessage(
                message_type="greeting",
                version="1.0",
                commands=["get_item_list", "add_variables"]
            )
        elif message.message_type == "command":
            if message.command.command_type == "get_item_list":
                return WcpSCMessage(
                    message_type="response",
                    response=WcpResponse(
                        response_type="get_item_list",
                        data=["item1", "item2"]
                    )
                )
            elif message.command.command_type == "add_variables":
                return WcpSCMessage(
                    message_type="response",
                    response=WcpResponse(
                        response_type="add_variables",
                        data=[DisplayedItemRef(id="var1"), DisplayedItemRef(id="var2")]
                    )
                )
        return WcpSCMessage(
            message_type="error",
            error={"error": "Unknown command", "message": "Command not recognized"}
        )

    def get_item_list(self) -> List[str]:
        """
        Request a list of displayed items from the server.

        :return: A list of item names.
        """
        message = WcpCSMessage(
            message_type="command",
            command=WcpCommand(command_type="get_item_list")
        )
        response = self.send_message(message)
        if response.message_type == "response" and response.response.response_type == "get_item_list":
            return response.response.data
        return []

    def add_variables(self, names: List[str]) -> List[DisplayedItemRef]:
        """
        Add variables to the view.

        :param names: A list of variable names to add.
        :return: A list of references to the added variables.
        """
        message = WcpCSMessage(
            message_type="command",
            command=WcpCommand(command_type="add_variables", args={"names": names})
        )
        response = self.send_message(message)
        if response.message_type == "response" and response.response.response_type == "add_variables":
            return response.response.data
        return []
