import pytest
import surfer
from libsurfer import Surfer

# Example VCD files to test with
TEST_FILES = [
    "examples/counter.vcd",
    "examples/counter2.vcd",
    "examples/picorv32.vcd",
    "examples/with_1_bit.vcd",
    "examples/with_8_bit.vcd"
]

@pytest.mark.parametrize("vcd_file", TEST_FILES)
def test_compare_implementations(vcd_file):
    # Load with python-surfer
    py_surfer = surfer.Surfer()
    py_result = py_surfer.load(vcd_file)
    
    # Load with libsurfer
    rust_surfer = Surfer()
    rust_result = rust_surfer.load(vcd_file)
    
    # Compare basic metadata
    assert py_result.metadata == rust_result.metadata
    
    # Compare signal counts
    py_signals = py_result.get_signals()
    rust_signals = rust_result.get_signals()
    assert len(py_signals) == len(rust_signals)
    
    # Compare signal names
    py_signal_names = {s.name for s in py_signals}
    rust_signal_names = {s.name for s in rust_signals}
    assert py_signal_names == rust_signal_names
    
    # Compare signal values at key timestamps
    for signal in py_signals:
        py_values = py_result.get_values(signal)
        rust_values = rust_result.get_values(signal)
        assert py_values == rust_values

def test_error_handling():
    # Test error handling consistency
    invalid_file = "nonexistent.vcd"
    
    # Python implementation
    with pytest.raises(FileNotFoundError):
        surfer.Surfer().load(invalid_file)
        
    # Rust implementation
    with pytest.raises(FileNotFoundError):
        Surfer().load(invalid_file)
