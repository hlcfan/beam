# Post-Request Script API Reference

This document is a practical reference for the **currently supported** post-request script API in Beam.

## `pm.response`

### Summary

`pm.response` exposes HTTP response data and metadata.

### Supported API

- `pm.response.status` -> HTTP status code (number)
- `pm.response.statusText` -> HTTP status text (string)
- `pm.response.json` -> pre-parsed JSON object, or `undefined` for non-JSON responses
- `pm.response.json()` -> backward-compatible function form of JSON access
- `pm.response.text()` -> raw response body (string)
- `pm.response.headers.get(name)` -> header value by name
- `pm.response.headers.has(name)` -> whether a header exists
- `pm.response.responseTime` -> response time in milliseconds
- `pm.response.bodySize` -> response body size in bytes

### Examples

```javascript
if (pm.response.status !== 200) {
    pm.fail("Expected 200, got " + pm.response.status);
}

const data = pm.response.json;
console.log("statusText:", pm.response.statusText);
console.log("content-type:", pm.response.headers.get("content-type"));
console.log("has x-request-id:", pm.response.headers.has("x-request-id"));
console.log("raw body:", pm.response.text());
console.log("timing:", pm.response.responseTime, "ms");
console.log("size:", pm.response.bodySize, "bytes");
```

### Non-JSON Behavior

When response body is not JSON:

- `pm.response.json` returns `undefined`
- accessing JSON does **not** throw
- a warning is logged

## `pm.extract(sourcePath, envVarName, defaultValue?)`

### Summary

Extracts data from response and writes it into the active environment.

### Supported Source Paths

- JSON dot-notation path, e.g. `$.data.token`
- JSON array indexing path, e.g. `$.items.0.id`
- response header path, e.g. `header:content-type`
- status keyword: `status`

### Return Value

- extracted value when found
- `defaultValue` when source is missing and default is provided
- `undefined` when source is missing and no default is provided

### Missing Path Behavior

If source is missing and no default is provided:

- logs a clear warning
- returns `undefined`
- skips environment update
- continues execution

### Examples

```javascript
// JSON field extraction
pm.extract("$.data.token", "auth_token");
pm.extract("$.user.id", "user_id");

// Array indexing
pm.extract("$.items.0.id", "first_item_id");

// Default fallback
pm.extract("$.meta.page", "current_page", "1");

// Header extraction
pm.extract("header:content-type", "content_type");
pm.extract("header:authorization", "auth_header");

// Status extraction
pm.extract("status", "last_status");

// Use returned value
const token = pm.extract("$.data.token", "auth_token");
console.log("Extracted token:", token);
```

## `pm.environment`

### Summary

Environment variable helper methods for script workflows.

### Supported API

- `pm.environment.set(key, value, type?)`
- `pm.environment.setAll({ key: value, ... })`
- `pm.environment.setIfPresent(key, value)`
- `pm.environment.has(key)`
- `pm.environment.clear()`

### Examples

```javascript
// Single set
pm.environment.set("token", pm.response.json.token);

// Set with optional type parameter (reserved for future typed environments)
pm.environment.set("count", pm.response.json.total, "number");

// Bulk set
pm.environment.setAll({
    token: pm.response.json.token,
    refresh_token: pm.response.json.refresh_token,
    expires_in: pm.response.json.expires_in,
});

// Conditional set
pm.environment.setIfPresent("nickname", pm.response.json.user.nickname);

// Existence check
if (pm.environment.has("token")) {
    console.log("token is present");
}

// Clear all variables
pm.environment.clear();
```

## `pm.test`

### Summary

Assertion helpers for common response validations.

### Supported API

- `pm.test(name, callback)` (existing callback style)
- `pm.test.status(expected)`
- `pm.test.statusOneOf([expected...])`
- `pm.test.json(path, expected, message?)`
- `pm.test.jsonExists(path)`
- `pm.test.header(name, expected)`
- `pm.test.responseTimeLessThan(ms)`

### Result Model

Each test result contains:

- `name`
- `passed`
- `expected`
- `actual`
- optional `error_message`

One-liner helpers auto-generate readable test names.

### Examples

```javascript
// Existing callback style (still supported)
pm.test("Status is 200", function () {
    return pm.response.status === 200;
});

// One-liner helpers
pm.test.status(200);
pm.test.statusOneOf([200, 201, 204]);
pm.test.json("$.success", true);
pm.test.jsonExists("$.data.id");
pm.test.header("content-type", "application/json");
pm.test.responseTimeLessThan(500);

// Custom failure message
pm.test.json("$.status", "active", "User must be active");
```

## `pm.fail(message)`

### Summary

Stops script execution immediately and marks script as failed.

### Behavior

- halts execution immediately
- keeps environment changes made before `pm.fail`
- records test result before halting if called inside a `pm.test()` callback
- surfaces failure reason in UI

### Example

```javascript
if (pm.response.status !== 200) {
    pm.fail("Expected 200, got " + pm.response.status);
}

// This line will not run if pm.fail is called above
pm.environment.set("post_fail", "unreachable");
```

## Console Output

The following console methods are surfaced in script results:

- `console.log(...)`
- `console.error(...)`
- `console.warn(...)`
- `console.info(...)`
- `console.debug(...)`

### Example

```javascript
console.log("Script started");
console.info("Response status:", pm.response.status);
console.warn("Optional warning");
console.error("Example error log");
console.debug("Debug payload:", pm.response.json);
```

## Execution Model

- script runs in a fresh sandbox context per request
- environment is snapshotted at script start
- environment writes are accumulated during execution
- changes are applied on success or controlled failure (`pm.fail`)
- test results and console output are sent to UI

### Guardrails

- timeout: 5 seconds
- memory limit: 64 MB heap

### Error Handling

- syntax error: script fails and error is shown in UI
- runtime exception: execution halts, partial env changes still apply
- timeout: script force-terminated, partial env changes still apply
- missing extract path (without default): warning + continue
- non-JSON JSON-access: returns `undefined` + warning

## Common Recipes

### Extract bearer token

```javascript
pm.extract("$.token", "auth_token");
```

### Chain a created resource ID

```javascript
pm.extract("$.id", "created_user_id");
```

### Assert and extract

```javascript
pm.test.status(201);
pm.extract("$.id", "last_created_id");
```

### Bulk extract after login

```javascript
const data = pm.response.json;
pm.environment.setAll({
    token: data.token,
    refresh_token: data.refresh_token,
    expires_in: data.expires_in,
});
```

### Basic shape checks

```javascript
pm.test.jsonExists("$.data.id");
pm.test.jsonExists("$.data.email");
pm.test.json("$.data.active", true);
```