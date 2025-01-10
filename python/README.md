# WCP (Waveform Communication Protocol) Python Client

This repository contains a Python implementation of a client for the WCP (Waveform Communication Protocol). The client is designed to interact with a WCP server, sending commands and receiving responses or events. The implementation uses Python's `dataclass` for clean and concise data structures, adheres to PEP8 standards, and includes a test suite and Sphinx-compatible documentation.

---

## Features

- **Dataclass-based Data Structures**: Clean and type-safe representation of WCP messages, commands, responses, and events.
- **Simulated Server Interaction**: Includes a placeholder method to simulate server responses for testing.
- **Test Suite**: Comprehensive unit tests using `unittest` to ensure functionality.
- **Sphinx Documentation**: Ready-to-use Sphinx-compatible documentation for easy integration into larger projects.
- **PEP8 Compliance**: Code adheres to Python's style guidelines for readability and maintainability.

---

## Installation

To use the WCP client, simply clone this repository and import the `WcpClient` class into your project.

```bash
git clone https://github.com/your-repo/wcp-client.git
cd wcp-client
```

No additional dependencies are required beyond Python 3.7+.

---

## Usage

### Example: Interacting with the WCP Server

Below is a simple example demonstrating how to use the `WcpClient` to interact with a WCP server.

```python
from wcp_client import WcpClient

# Initialize the client
client = WcpClient("http://localhost:8080")

# Get a list of displayed items
items = client.get_item_list()
print("Displayed Items:", items)

# Add variables to the view
variables = client.add_variables(["var1", "var2"])
print("Added Variables:", [var.id for var in variables])
```

### Output

```
Displayed Items: ['item1', 'item2']
Added Variables: ['var1', 'var2']
```

---

## Testing

The repository includes a test suite to verify the functionality of the `WcpClient`. To run the tests, execute the following command:

```bash
python -m unittest discover
```

---

## Documentation

The project includes Sphinx-compatible documentation. To generate the documentation:

1. Install Sphinx:
   ```bash
   pip install sphinx
   ```
2. Navigate to the `docs` directory and build the documentation:
   ```bash
   cd docs
   make html
   ```
3. Open `docs/_build/html/index.html` in your browser to view the documentation.

---

## Data Structures

The following data structures are defined in the implementation:

- **`DisplayedItemRef`**: Represents a reference to a displayed item.
- **`ItemInfo`**: Represents detailed information about a displayed item.
- **`WcpCommand`**: Represents a command sent from the client to the server.
- **`WcpResponse`**: Represents a response sent from the server to the client.
- **`WcpEvent`**: Represents an event sent from the server to the client.
- **`WcpCSMessage`**: Represents a message sent from the client to the server.
- **`WcpSCMessage`**: Represents a message sent from the server to the client.

---

## Notes

1. The `_simulate_server_response` method is a placeholder for actual server communication. Replace it with HTTP or WebSocket communication for real-world usage.
2. The implementation assumes JSON serialization for messages. Extend it to use your preferred communication protocol.
3. The test suite uses `unittest` for simplicity. You can replace it with `pytest` if preferred.

---

## Contributing

Contributions are welcome! Please open an issue or submit a pull request for any improvements or bug fixes.

---

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

---

## Contact

For questions or feedback, please contact [Your Name] at [your.email@example.com].
