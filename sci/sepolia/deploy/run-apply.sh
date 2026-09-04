#!/usr/bin/env bash
set -a
source sci/sepolia/deploy/.env
SEQ1_P2P_KEY=7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6
SEQ2_P2P_KEY=47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a
BUILDER_P2P_KEY=2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6
BUILDER_ENODE_ID=3255458e24278e31d5940f304b16300fdff3f6efd3e2a030b5818310ac67af45e28d057e6a332d07e0c5ab09d6947fd4eed1a646edbf224e2d2fec6f49f90abc
set +a
cd ~/sci-dev/sci-chain
rm -f .sepolia/l2/configs/*.json 2>/dev/null
docker rm -f setup-l2-sepolia 2>/dev/null
docker run --rm --name setup-l2-sepolia --network host \
  -e L1_RPC_URL=http://127.0.0.1:8645 \
  -e L1_CHAIN_ID=11155111 -e L2_CHAIN_ID=42001 \
  -e OUTPUT_DIR=/output -e TEMPLATE_DIR=/templates \
  -e DEPLOYER_ADDR -e DEPLOYER_KEY \
  -e SEQUENCER_ADDR -e BATCHER_ADDR -e PROPOSER_ADDR -e CHALLENGER_ADDR \
  -e SEQ1_P2P_KEY -e SEQ2_P2P_KEY -e BUILDER_P2P_KEY -e BUILDER_ENODE_ID \
  -v "$PWD/.sepolia/l2/configs:/output" \
  --entrypoint /usr/local/bin/setup-l2.sh devnet-setup:local
echo "APPLY_EXIT=$?"
