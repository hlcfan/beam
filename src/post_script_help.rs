pub const POST_SCRIPT_API_HELP_MARKDOWN: &str = r#"### Supported Post Script APIs

Scripts run with a small Postman-style runtime. Environment writes require an active environment.

#### pm.response

##### status

```javascript
if (pm.response.status === 200) console.log("request ok");
```

##### statusText

```javascript
console.log("status text:", pm.response.statusText);
```

##### responseTime

```javascript
console.log("response time:", pm.response.responseTime + "ms");
```

##### bodySize

```javascript
console.log("body size:", pm.response.bodySize + " bytes");
```

##### headers.get(name)

```javascript
const requestId = pm.response.headers.get("x-request-id");
```

##### headers.has(name)

```javascript
pm.test("etag header exists", () => pm.response.headers.has("etag"));
```

##### text()

```javascript
console.log(pm.response.text());
```
##### json()

```javascript
const token = pm.response.json().token;
```

#### pm.environment

##### get(key)

```javascript
const baseUrl = pm.environment.get("base_url");
```

##### has(key)

```javascript
if (!pm.environment.has("token")) console.warn("token missing");
```

##### set(key, value)

```javascript
pm.environment.set("token", pm.response.json().token);
```

##### setAll(obj)

```javascript
pm.environment.setAll({ token: pm.response.json().token, user_id: pm.response.json().user.id });
```

##### setIfPresent(key, value)

```javascript
pm.environment.setIfPresent("refresh_token", pm.response.json().refresh_token);
```

##### unset(key)

```javascript
pm.environment.unset("token");
```

##### clear()

```javascript
pm.environment.clear();
```

#### Assertions

##### test(name, fn)

```javascript
pm.test("token exists", () => !!pm.response.json().token);
```

##### status(code)

```javascript
pm.test.status(200);
```

##### statusOneOf([codes])

```javascript
pm.test.statusOneOf([200, 201, 204]);
```

##### json(path, expected)

```javascript
pm.test.json("$.user.role", "admin");
```

##### jsonExists(path)

```javascript
pm.test.jsonExists("$.data.items.0.id");
```

##### header(name, expected)

```javascript
pm.test.header("content-type", "application/json");
```

##### responseTimeLessThan(ms)

```javascript
pm.test.responseTimeLessThan(500);
```

##### fail(message)

```javascript
if (!pm.response.json().token) pm.fail("Missing token in response");
```

#### Utilities

##### extract(sourcePath, envVarName, defaultValue)

```javascript
pm.extract("$.data.token", "token", "guest-token");
```

#### console

##### log(...)

```javascript
console.log("user id:", pm.response.json().user.id);
```
##### info(...)

```javascript
console.info("saved token to environment");
```

##### warn(...)

```javascript
console.warn("response body was not JSON");
```

##### error(...)

```javascript
console.error("token lookup failed");
```

##### debug(...)

```javascript
console.debug("raw response:", pm.response.text());
```
"#;
