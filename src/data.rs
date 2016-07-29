use serde_json::Value;
use serde_json::de::from_str;

#[derive(Clone)]
pub enum Data {
    JSON(Value),
    Binary(Vec<u8>),
}

#[doc(hidden)]
pub fn encode_data(data: Vec<Data>) -> (Value, Vec<Vec<u8>>) {
    let mut json = vec![];
    let mut binary = vec![];
    let mut placeholder_num = 0;

    for value in data {
        json.push(match value {
            Data::JSON(v) => v,
            Data::Binary(b) => {
                binary.push(b);
                placeholder_num = placeholder_num + 1;
                placeholder(placeholder_num)
            }
        })
    }

    (Value::Array(json), binary)
}

fn placeholder(num: usize) -> Value {
    from_str(&format!("{{\"_placeholder\":true,\"num\": {}}}", num)).unwrap()
}
