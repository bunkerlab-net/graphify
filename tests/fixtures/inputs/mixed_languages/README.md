# Mixed Language Demo

This corpus demonstrates a simple three-tier stack:

- **Python** (`server.py`) — HTTP server exposing `/health` and `/users` endpoints.
- **TypeScript** (`client.ts`) — Browser/Node client consuming those endpoints.
- **Go** (`proxy.go`) — Reverse proxy sitting in front of the Python server.

## Architecture

```
Browser → TypeScript client
            ↓
          Go proxy (:9090)
            ↓
          Python server (:8080)
```

## Running

```bash
# Start the Python server
python server.py

# Start the Go proxy
go run proxy.go

# Use the TypeScript client from a browser or Deno
```
