cargo run -- \
    node --datadir target/d2 --chain assets/genesis.json \
    --instance 2 --trusted-peers $(cat /tmp/enode) \
    --producer-url ws://localhost:8546


