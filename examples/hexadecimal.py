import surfer_waveform

translators = ["HexTranslator"]


class HexTranslator(surfer_waveform.BasicTranslator):
    name = "Hexadecimal (Python)"

    @staticmethod
    def basic_translate(num_bits: int, value: str):
        try:
            h = hex(int(value))[2:]
            return f"0x{h.zfill(num_bits // 4)}", surfer_waveform.ValueKind.Normal()
        except ValueError:
            return value, surfer_waveform.ValueKind.Warn()
