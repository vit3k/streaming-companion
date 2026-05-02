#!/bin/bash

# Test script for SteamGridDB poster integration
# Usage: ./test_poster_api.sh

set -e

BASE_URL="http://127.0.0.1:7878"

echo "=========================================="
echo "Testing SteamGridDB Poster Integration"
echo "=========================================="
echo ""

# Test 1: Get all games
echo "[TEST 1] Fetching all games..."
GAMES=$(curl -s "$BASE_URL/games")
echo "Games response (first 500 chars):"
echo "$GAMES" | head -c 500
echo ""
echo ""

# Extract first Steam game ID
STEAM_GAME_ID=$(echo "$GAMES" | grep -o '"id":"[^"]*"' | grep -o '[^"]*"' | head -1 | tr -d '"')

if [ -z "$STEAM_GAME_ID" ]; then
    echo "ERROR: No games found in response"
    exit 1
fi

echo "[TEST 2] Found Steam game with ID: $STEAM_GAME_ID"
echo ""

# Test 2: Get poster for the first game
echo "[TEST 3] Fetching poster for game ID: $STEAM_GAME_ID..."
POSTER=$(curl -s "$BASE_URL/games/$STEAM_GAME_ID/poster")
echo "Poster response:"
echo "$POSTER" | jq . 2>/dev/null || echo "$POSTER"
echo ""

# Test 3: Verify cache was created
echo "[TEST 4] Checking cache directory..."
CACHE_DIR="$HOME/.cache/suspend-web"
if [ -d "$CACHE_DIR" ]; then
    echo "Cache directory exists: $CACHE_DIR"
    echo "Cached files:"
    ls -lh "$CACHE_DIR" 2>/dev/null || echo "  (empty)"
else
    echo "Cache directory not yet created"
fi
echo ""

echo "=========================================="
echo "Tests complete!"
echo "=========================================="
