use std::io::{BufReader, BufWriter, MemWriter, SeekSet};
use std::vec::Vec;
use test::Bencher;


fn roundtrip(bytes: &[u8]) {
    info!("Roundtrip Ari of size {}", bytes.len());
    let mut e = super::table::ByteEncoder::new(MemWriter::new());
    e.write(bytes).unwrap();
    let (e, r) = e.finish();
    r.unwrap();
    let encoded = e.unwrap();
    debug!("Roundtrip input {:?} encoded {:?}", bytes, encoded);
    let mut d = super::ByteDecoder::new(BufReader::new(encoded.as_slice()));
    let decoded = d.read_to_end().unwrap();
    assert_eq!(bytes.as_slice(), decoded.as_slice());
}

fn encode_binary(bytes: &[u8], model: &mut super::bin::Model, factor: uint) -> Vec<u8> {
    let mut encoder = super::Encoder::new(MemWriter::new());
    for &byte in bytes.iter() {
        for i in range(0,8) {
            let bit = (byte & (1<<i)) != 0;
            encoder.encode(bit, model).unwrap();
            model.update(bit, factor);
        }
    }
    let (writer, err) = encoder.finish();
    err.unwrap();
    writer.unwrap()
}

fn roundtrip_binary(bytes: &[u8], factor: uint) {
    let mut bm = super::bin::Model::new_flat(super::range_default_threshold >> 3);
    let output = encode_binary(bytes, &mut bm, factor);
    bm.reset_flat();
    let mut decoder = super::Decoder::new(BufReader::new(output.as_slice()));
    for &byte in bytes.iter() {
        let mut value = 0u8;
        for i in range(0,8) {
            let bit = decoder.decode(&bm).unwrap();
            bm.update(bit, factor);
            value += (bit as u8)<<i;
        }
        assert_eq!(value, byte);
    }
}

fn roundtrip_term(bytes1: &[u8], bytes2: &[u8]) {
    let mw = MemWriter::new();
    let mw = {
        let mut e = super::table::ByteEncoder::new(mw);
        e.write(bytes1).unwrap();
        let (stream, rez) = e.finish();
        rez.unwrap();
        stream
    };
    let mw = {
        let mut e = super::table::ByteEncoder::new(mw);
        e.write(bytes2).unwrap();
        let (stream, rez) = e.finish();
        rez.unwrap();
        stream
    };
    let encoded = mw.unwrap();
    debug!("Roundtrip term input {:?}:{:?} encoded {:?}", bytes1, bytes2, encoded);
    let br = BufReader::new(encoded.as_slice());
    let br = {
        let mut d = super::ByteDecoder::new(br);
        let decoded = d.read_to_end().unwrap();
        assert_eq!(bytes1.as_slice(), decoded.as_slice());
        let (stream, err) = d.finish();
        err.unwrap();
        stream
    };
    {
        let mut d = super::ByteDecoder::new(br);
        let decoded = d.read_to_end().unwrap();
        assert_eq!(bytes2.as_slice(), decoded.as_slice());
        let (stream, err) = d.finish();
        err.unwrap();
        stream
    };
}

fn roundtrip_proxy(bytes: &[u8]) {
    // prepare data
    let factor0 = 3;
    let factor1 = 5;
    let update0 = 10;
    let update1 = 5;
    let threshold = super::range_default_threshold >> 3;
    let mut t0 = super::table::Model::new_flat(16, threshold);
    let mut t1 = super::table::Model::new_flat(16, threshold);
    let mut b0 = super::bin::Model::new_flat(threshold);
    let mut b1 = super::bin::Model::new_flat(threshold);
    // encode (high 4 bits with the proxy table, low 4 bits with the proxy binary)
    let mut encoder = super::Encoder::new(MemWriter::new());
    for &byte in bytes.iter() {
        let high = (byte>>4) as uint;
        {
            let proxy = super::table::SumProxy::new(2, &t0, 1, &t1, 0);
            encoder.encode(high, &proxy).unwrap();
        }
        t0.update(high, update0, 1);
        t1.update(high, update1, 1);
        for i in range(0,4) {
            let bit = (byte & (1<<i)) != 0;
            {
                let proxy = super::bin::SumProxy::new(1, &b0, 1, &b1, 1);
                encoder.encode(bit, &proxy).unwrap();
            }
            b0.update(bit, factor0);
            b1.update(bit, factor1);
        }
    }
    let (writer, err) = encoder.finish();
    err.unwrap();
    let buffer = writer.unwrap();
    // decode
    t0.reset_flat();
    t1.reset_flat();
    b0.reset_flat();
    b1.reset_flat();
    let mut decoder = super::Decoder::new(BufReader::new(buffer.as_slice()));
    for &byte in bytes.iter() {
        let high = {
            let proxy = super::table::SumProxy::new(2, &t0, 1, &t1, 0);
            decoder.decode(&proxy).unwrap()
        };
        t0.update(high, update0, 1);
        t1.update(high, update1, 1);
        let mut value = (high<<4) as u8;
        for i in range(0,4) {
            let bit = {
                let proxy = super::bin::SumProxy::new(1, &b0, 1, &b1, 1);
                decoder.decode(&proxy).unwrap()
            };
            value += (bit as u8)<<i;
            b0.update(bit, factor0);
            b1.update(bit, factor1);
        }
        assert_eq!(value, byte);
    }
}

#[test]
fn roundtrips() {
    roundtrip(bytes!("abracadabra"));
    roundtrip(bytes!(""));
    roundtrip(include_bin!("../../data/test.txt"));
}

#[test]
fn roundtrips_binary() {
    roundtrip_binary(bytes!("abracadabra"), 1);
    roundtrip_binary(include_bin!("../../data/test.txt"), 5);
}

#[test]
fn roundtrips_term() {
    roundtrip_term(bytes!("abra"), bytes!("cadabra"));
}

#[test]
fn roundtrips_proxy() {
    roundtrip_proxy(bytes!("abracadabra"));
    roundtrip_proxy(include_bin!("../../data/test.txt"));
}

#[bench]
fn compress_speed(bh: &mut Bencher) {
    let input = include_bin!("../../data/test.txt");
    let mut storage = Vec::from_elem(input.len(), 0u8);
    bh.iter(|| {
        let mut w = BufWriter::new(storage.as_mut_slice());
        w.seek(0, SeekSet).unwrap();
        let mut e = super::ByteEncoder::new(w);
        e.write(input).unwrap();
    });
    bh.bytes = input.len() as u64;
}
