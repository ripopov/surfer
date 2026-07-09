use ftr_parser::parse::parse_ftr;
use ftr_parser::types::StreamId;
use std::path::PathBuf;

#[allow(unused_must_use)]
fn main() {
    let path = PathBuf::from("./examples/test.ftr");

    let mut ftr = parse_ftr(path).unwrap();

    println!("Timescale: {:?}", ftr.time_scale);
    println!("Max timestamp: {:?}", ftr.max_timestamp);

    println!("Dictionary: ");
    for entry in &ftr.str_dict {
        println!("key: {}, value: {}", entry.0, entry.1);
    }
    println!();

    println!("Streams: ");
    for stream in &ftr.tx_streams {
        println!("{:?}", stream);
    }
    println!();

    ftr.load_stream_into_memory(StreamId(1));
    ftr.load_stream_into_memory(StreamId(2));
    ftr.load_stream_into_memory(StreamId(3));

    println!("Generators: ");
    for gen in &ftr.tx_generators {
        println!("{:?}", gen);
    }

    println!();

    println!("Streams: ");
    for stream in &ftr.tx_streams {
        println!("{:?}", stream);
    }
}
