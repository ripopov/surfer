import pytest
from python_surfer import (
    DisplayedItemRef,
    ItemInfo,
    WcpCommand,
    WcpResponse,
    WcpEvent,
    WcpCSMessage,
    WcpSCMessage,
    WcpClient,
)


@pytest.fixture
def client():
    """Fixture to initialize the WCP client for testing."""
    return WcpClient("http://localhost:8080")


def test_displayed_item_ref():
    """Test the DisplayedItemRef dataclass."""
    item_ref = DisplayedItemRef(id="item1")
    assert item_ref.id == "item1"


def test_item_info():
    """Test the ItemInfo dataclass."""
    item_info = ItemInfo(id="item1", name="Item 1", color="red")
    assert item_info.id == "item1"
    assert item_info.name == "Item 1"
    assert item_info.color == "red"


def test_wcp_command():
    """Test the WcpCommand dataclass."""
    command = WcpCommand(command_type="get_item_list", args={"param": "value"})
    assert command.command_type == "get_item_list"
    assert command.args == {"param": "value"}


def test_wcp_response():
    """Test the WcpResponse dataclass."""
    response = WcpResponse(response_type="get_item_list", data=["item1", "item2"])
    assert response.response_type == "get_item_list"
    assert response.data == ["item1", "item2"]


def test_wcp_event():
    """Test the WcpEvent dataclass."""
    event = WcpEvent(event_type="item_added", data={"id": "item1"})
    assert event.event_type == "item_added"
    assert event.data == {"id": "item1"}


def test_wcp_cs_message():
    """Test the WcpCSMessage dataclass."""
    command = WcpCommand(command_type="get_item_list")
    message = WcpCSMessage(message_type="command", command=command)
    assert message.message_type == "command"
    assert message.command == command


def test_wcp_sc_message():
    """Test the WcpSCMessage dataclass."""
    response = WcpResponse(response_type="get_item_list", data=["item1", "item2"])
    message = WcpSCMessage(message_type="response", response=response)
    assert message.message_type == "response"
    assert message.response == response


def test_client_initialization(client):
    """Test the initialization of the WCP client."""
    assert client.server_url == "http://localhost:8080"


def test_get_item_list(client):
    """Test the get_item_list method of the WCP client."""
    items = client.get_item_list()
    assert items == ["item1", "item2"]


def test_add_variables(client):
    """Test the add_variables method of the WCP client."""
    variables = client.add_variables(["var1", "var2"])
    assert len(variables) == 2
    assert variables[0].id == "var1"
    assert variables[1].id == "var2"


def test_send_message_greeting(client):
    """Test sending a greeting message to the server."""
    message = WcpCSMessage(message_type="greeting")
    response = client.send_message(message)
    assert response.message_type == "greeting"
    assert response.version == "1.0"
    assert response.commands == ["get_item_list", "add_variables"]


def test_send_message_command_get_item_list(client):
    """Test sending a command to get the item list."""
    command = WcpCommand(command_type="get_item_list")
    message = WcpCSMessage(message_type="command", command=command)
    response = client.send_message(message)
    assert response.message_type == "response"
    assert response.response.response_type == "get_item_list"
    assert response.response.data == ["item1", "item2"]


def test_send_message_command_add_variables(client):
    """Test sending a command to add variables."""
    command = WcpCommand(command_type="add_variables", args={"names": ["var1", "var2"]})
    message = WcpCSMessage(message_type="command", command=command)
    response = client.send_message(message)
    assert response.message_type == "response"
    assert response.response.response_type == "add_variables"
    assert len(response.response.data) == 2
    assert response.response.data[0].id == "var1"
    assert response.response.data[1].id == "var2"


def test_send_message_unknown_command(client):
    """Test sending an unknown command to the server."""
    command = WcpCommand(command_type="unknown_command")
    message = WcpCSMessage(message_type="command", command=command)
    response = client.send_message(message)
    assert response.message_type == "error"
    assert response.error == {"error": "Unknown command", "message": "Command not recognized"}


def test_send_message_invalid_message_type(client):
    """Test sending a message with an invalid message type."""
    message = WcpCSMessage(message_type="invalid_type")
    response = client.send_message(message)
    assert response.message_type == "error"
    assert response.error == {"error": "Unknown command", "message": "Command not recognized"}
