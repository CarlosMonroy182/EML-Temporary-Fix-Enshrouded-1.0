use kfc::container::{KFCReader, KFCReaderOptions};
use kfc::guid::ResourceId;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let r = KFCReader::new_with_options(&a[1], "enshrouded", KFCReaderOptions { kfc_extension: if a.len() > 3 { a[3].as_str() } else { "kfc" }, ..Default::default() }).unwrap();
    let mut c = r.into_cursor().unwrap();
    let id = ResourceId::parse_qualified(&a[2]).unwrap();
    let d = c.read_resource(&id).unwrap().unwrap();
    println!("{} bytes", d.len());
    for ch in d.chunks(16) { println!("{}", ch.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")); }
}
