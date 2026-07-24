#!/bin/bash
set -e

echo "Building Quaxar Docker image..."
docker compose -f docker-compose-testnet.yml build

echo "Loading snapshot into Quaxar NodeStore..."
docker run --rm \
  -v quaxar-testnet-data:/var/lib/xrpld \
  -v $(pwd)/xrpld-testnet.cfg:/etc/xrpld/xrpld.cfg:ro \
  -v ~/Downloads/testnet-19288379.xrpls:/snapshot.xrpls:ro \
  quaxar-testnet \
  --conf /etc/xrpld/xrpld.cfg load-snapshot --input /snapshot.xrpls

echo "Snapshot loaded successfully. Starting Quaxar Testnet node..."
docker compose -f docker-compose-testnet.yml up -d

echo "Node is running! You can view logs with: docker logs -f quaxar-testnet"
