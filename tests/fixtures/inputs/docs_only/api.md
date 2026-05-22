# API Reference

All endpoints accept and return JSON. Authentication uses Bearer tokens.

## Topics

### `POST /topics`

Create a new topic.

**Body:**
```json
{
  "name": "events",
  "partitions": 12,
  "replication_factor": 3,
  "retention_hours": 168
}
```

**Response:** `201 Created` with topic metadata.

### `GET /topics/{name}`

Get topic metadata including partition count and replica assignment.

### `DELETE /topics/{name}`

Delete a topic. In-progress consumers will receive `TopicDeletedException`.

## Producers

### `POST /topics/{name}/messages`

Publish one or more messages.

**Body:**
```json
{
  "messages": [
    {"key": "user-42", "value": "eyJldmVudCI6ICJsb2dpbiJ9", "headers": {}}
  ]
}
```

Values are base64-encoded bytes.

## Consumers

### `POST /consumers`

Create a consumer in a group.

### `GET /consumers/{id}/messages`

Poll for messages. Returns up to `max_records` messages across assigned partitions.

### `POST /consumers/{id}/offsets`

Commit offsets.
