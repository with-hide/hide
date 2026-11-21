//! Prints what `contracts/peer-link.json` must say (`contracts/README.md`,
//! Peer link).

fn main() {
    let text = serde_json::to_string_pretty(&hide_peer_protocol::contract::shape())
        .expect("the contract is JSON");
    println!("{text}");
}
