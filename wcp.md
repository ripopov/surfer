# WCP Protocol Documentation

## Overview
The WCP (Waveform Communication Protocol) is designed to facilitate the communication between a client (which may be a GUI or another system) and a server (named "surfer") handling waveform data. The protocol allows the client to send commands for manipulating waveforms, and the server responds with messages indicating the success, failure, or status of those commands. 

## Key Components

### 1. **Messages**
The WCP protocol uses two main types of messages:
- **WcpCSMessage**: Messages from the client to the server.
- **WcpSCMessage**: Messages from the server to the client.

### 2. **WcpCSMessage**
The `WcpCSMessage` enum contains the following variants, which represent different types of messages that clients can send to the server:

- **greeting**
    - **Fields**:
        - `version: String`: The version of the WCP being used.
        - `commands: Vec<String>`: The list of commands the server supports.

- **command(WcpCommand)**: Contains the specific command to be executed by the server.

### 3. **WcpCommand**
The `WcpCommand` enum defines the commands supported by the protocol:

- **get_item_list**: Requests a list of displayed items.
- **get_item_info { ids: Vec<DisplayedItemRef> }**: Requests detailed information about specific items by their IDs.
- **add_variables { names: Vec<String> }**: Adds variables to the view based on the provided names.
- **add_scope { scope: String }**: Adds all variables in a specific scope to the view.
- **reload**: Reloads the waveform if permissible.
- **set_viewport_to { timestamp: BigInt }**: Moves the viewport to a specified timestamp.
- **set_item_color { id: DisplayedItemRef, color: String }**: Changes the color of an item.
- **remove_items { ids: Vec<DisplayedItemRef> }**: Removes the specified items from the view.
- **focus_item { id: DisplayedItemRef }**: Sets the specified item as the focused item.
- **clear**: Clears all displayed items.
- **load { source: String }**: Loads a waveform from the specified file or URL.
- **zoom_to_fit { viewport_idx: usize }**: Zooms out to fit the entire waveform in the view.
- **shutdowmn**: Indicates that the client wants to shut down the server.

### 4. **WcpSCMessage**
The `WcpSCMessage` enum represents the messages sent from the server back to the client. Possible variants include:

- **greeting { version: String, commands: Vec<String> }**: A greeting message from the server acknowledging the client's greeting.
- **response(WcpResponse)**: A standard response to a command.
- **error { error: String, arguments: Vec<String>, message: String }**: An error notification when a command fails.
- **event(WcpEvent)**: Events that the server sends to the client.

### 5. **WcpResponse**
The `WcpResponse` enum contains various responses that the server can send in reply to client commands:
- **get_item_list(Vec<String>)**: Responds with a list of item names currently displayed.
- **get_item_info(Vec<ItemInfo>)**: Responds with item information based on the IDs requested.
- **add_variables(Vec<DisplayedItemRef>)**: Responds with the IDs of added variables.
- **add_scope(Vec<DisplayedItemRef>)**: Responds with the IDs of variables added in the specified scope.
- **ack**: Acknowledgment response indicating successful command processing.

### 6. **WcpEvent**
The `WcpEvent` enum denotes events that the server can raise, such as:
- **waveforms_loaded**: Indicates that the waveforms have been successfully loaded.

## Command and Response Summary

| Command                           | Expected Response                                    |
|-----------------------------------|-----------------------------------------------------|
| **greeting**                      | `greeting { version, commands }`                   |
| **get_item_list**                | `response(WcpResponse::get_item_list(Vec<String>))`|
| **get_item_info { ids }**        | `response(WcpResponse::get_item_info(Vec<ItemInfo>))` or `error` |
| **add_variables { names }**      | `response(WcpResponse::add_variables(Vec<DisplayedItemRef>))` or `error` |
| **add_scope { scope }**          | `response(WcpResponse::add_scope(Vec<DisplayedItemRef>))` or `error` |
| **reload**                        | `response(WcpResponse::ack)`                        |
| **set_viewport_to { timestamp }**| `response(WcpResponse::ack)`                        |
| **set_item_color { id, color }** | `response(WcpResponse::ack)` or `error`            |
| **remove_items { ids }**         | `response(WcpResponse::ack)`                        |
| **focus_item { id }**            | `response(WcpResponse::ack)` or `error`            |
| **clear**                         | `response(WcpResponse::ack)`                        |
| **load { source }**              | `response(WcpResponse::ack)` or `error`            |
| **zoom_to_fit { viewport_idx }** | `response(WcpResponse::ack)`                        |
| **shutdowmn**                    | `response(WcpResponse::ack)` or no response         |

## Events
Events are sent from the server to the client to update the client about state changes:
- **waveforms_loaded**: A notification sent when waveforms are loaded successfully.

## Conclusion
The WCP system implements standard command-response patterns, enabling rich interaction between clients and the surfer server. This documentation serves as a guide for developers looking to understand or implement features utilizing the WCP protocol, presenting a clear picture of the available commands, expected responses, and event types.
