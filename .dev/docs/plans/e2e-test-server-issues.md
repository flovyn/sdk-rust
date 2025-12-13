# E2E Test Server Configuration Issues

## Goal

Run E2E tests for the Rust SDK using a containerized Flovyn server with:
1. Security enabled (`FLOVYN_SECURITY_ENABLED=true`)
2. REST API calls to create tenant and worker token
3. Self-signed JWT tokens for authentication (no external OIDC provider needed)

---

## Root Cause Analysis

### Issue 1: OAuth2 Client Auto-Configuration Requires GitHub Credentials

**Error:**
```
Client id of registration 'github' must not be empty.
```

**Root Cause:**

The server has `spring-boot-starter-oauth2-client` dependency (line 39 in `build.gradle.kts`):
```kotlin
// Spring Security OAuth2 Client (for SSO with Google/GitHub)
implementation("org.springframework.boot:spring-boot-starter-oauth2-client")
```

This dependency triggers Spring Boot's `OAuth2ClientAutoConfiguration` which:
1. Scans for `spring.security.oauth2.client.registration.*` properties
2. Auto-configures well-known providers (GitHub, Google, Facebook, Okta)
3. For GitHub specifically, Spring Boot has built-in provider metadata but still **requires** `client-id` and `client-secret`

**Why it's triggered even without explicit GitHub config:**
- Spring Boot auto-detects the OAuth2 client starter
- `OAuth2ClientAutoConfiguration` runs unconditionally when the dependency is present
- The auto-configuration attempts to initialize a default `ClientRegistrationRepository`
- Even with an empty `spring.security.oauth2.client.registration` section, Spring Boot's `OAuth2ClientPropertiesMapper` validates that any referenced provider has credentials

**Evidence:**
- Login templates exist: `server/app/src/main/resources/templates/login.html` references `/oauth2/authorization/github`
- This suggests GitHub SSO was planned but never fully implemented
- The dependency was added prematurely before actual SSO implementation

**Solution Options:**

| Option | Complexity | Impact |
|--------|------------|--------|
| A. Remove `spring-boot-starter-oauth2-client` dependency | S | Removes unused SSO capability |
| B. Exclude auto-configuration via `spring.autoconfigure.exclude` | S | Keep dependency for future use |
| C. Provide dummy credentials for non-production | M | Adds unnecessary configuration |

**Recommended: Option B** - Exclude the auto-configuration in application.yml:
```yaml
spring:
  autoconfigure:
    exclude:
      - org.springframework.boot.autoconfigure.security.oauth2.client.servlet.OAuth2ClientAutoConfiguration
```

This keeps the dependency available for future SSO implementation while preventing auto-configuration failures.

---

### Issue 2: Security Disabled Mode Doesn't Parse JWT

**Error:**
```
401 Unauthorized - Authentication required. Please provide a valid JWT token with userId claim.
Missing request attribute 'userId' of type UUID
```

**Root Cause:**

When `flovyn.security.enabled=false`, `SecurityConfig.kt` uses `securityFilterChainPermitAll()`:

```kotlin
@Bean
@ConditionalOnProperty(
    name = ["flovyn.security.enabled"],
    havingValue = "false",
    matchIfMissing = true,
)
fun securityFilterChainPermitAll(http: HttpSecurity): SecurityFilterChain {
    http
        .csrf { it.disable() }
        .authorizeHttpRequests { auth ->
            auth.anyRequest().permitAll()
        }
    // NO FILTERS ADDED - JWT is never parsed!
    return http.build()
}
```

Compare with `securityFilterChainWithAuth()` (when security enabled):
```kotlin
// Adds UserAuthenticationFilter which extracts userId from JWT
http.addFilterAfter(UserAuthenticationFilter(), BearerTokenAuthenticationFilter::class.java)
```

**Why controllers fail:**
- Controllers use `@RequestAttribute("userId")` which is populated by `UserAuthenticationFilter`
- Without the filter, the attribute is never set
- Spring throws `MissingRequestValueException`

**The design flaw:**
- "Security disabled" was intended to mean "no authentication required"
- But it inadvertently also means "no JWT parsing at all"
- This breaks controllers that depend on `userId` even when authentication is optional

**Solution Options:**

| Option | Complexity | Impact |
|--------|------------|--------|
| A. Add `UserAuthenticationFilter` to permit-all chain | S | Parses JWT if present, doesn't require it |
| B. Make `userId` optional in controllers | L | Major refactoring, changes API contracts |
| C. Create separate "test mode" filter | M | Adds complexity |

**Recommended: Option A** - Modify `securityFilterChainPermitAll()` to still parse JWTs:

```kotlin
fun securityFilterChainPermitAll(http: HttpSecurity): SecurityFilterChain {
    http
        .csrf { it.disable() }
        .authorizeHttpRequests { auth ->
            auth.anyRequest().permitAll()
        }
        // Parse JWT if present (but don't require authentication)
        .oauth2ResourceServer { oauth2 ->
            oauth2.jwt { }
        }

    // Extract userId from JWT if present
    http.addFilterAfter(UserAuthenticationFilter(), BearerTokenAuthenticationFilter::class.java)

    return http.build()
}
```

---

### Issue 3: Security Enabled Mode Validates JWT Signature Against OIDC Provider

**Error:**
```
401 - Signed JWT rejected: Invalid signature
```

**Root Cause:**

The server's `application.yml` configures JWT validation using an issuer URI:
```yaml
spring:
  security:
    oauth2:
      resourceserver:
        jwt:
          issuer-uri: ${JWT_ISSUER_URI:http://localhost:3000}
```

When `oauth2ResourceServer { jwt { } }` is used in `SecurityConfig.kt`, Spring Security:
1. Fetches OIDC discovery document from `${issuer-uri}/.well-known/openid-configuration`
2. Extracts JWKS URI from the discovery document
3. Fetches public keys from JWKS endpoint
4. Validates JWT signatures against those public keys

**Why self-signed JWTs fail:**
- E2E tests generate JWTs signed with a test RSA private key
- The corresponding public key is NOT in the OIDC provider's JWKS
- Spring Security rejects the token because signature verification fails

**The fundamental problem:**
- The server is designed to validate JWTs against an external OIDC provider (Better Auth)
- E2E tests need to generate their own JWTs without running an OIDC provider
- There's no built-in way to configure a local public key alongside issuer-uri validation

**Solution Options:**

| Option | Complexity | Impact |
|--------|------------|--------|
| A. Add `jwt.skip-signature-verification` test mode | M | Parses JWT without validating signature |
| B. Support `jwt.public-key-location` override | M | Requires mounting keys into containers |
| C. Run mock OIDC server in tests | L | Adds infrastructure complexity |
| D. Configure custom `JwtDecoder` bean | S | Cleanest solution |

**Recommended: Option D** - Provide a conditional `JwtDecoder` bean that skips signature verification:

```kotlin
@Bean
@ConditionalOnProperty(
    name = ["flovyn.security.jwt.skip-signature-verification"],
    havingValue = "true",
)
fun unsafeJwtDecoder(): JwtDecoder {
    return JwtDecoder { token ->
        // Parse JWT without signature verification
        val parts = token.split(".")
        require(parts.size == 3) { "Invalid JWT format" }

        val payload = String(Base64.getUrlDecoder().decode(parts[1]))
        val claims = ObjectMapper().readValue(payload, Map::class.java)

        Jwt.withTokenValue(token)
            .headers { it["alg"] = "RS256" }
            .claims { it.putAll(claims) }
            .build()
    }
}
```

This should only be enabled in non-production environments (enforce via profile check).

---

## Implementation Plan

### Phase 1: Fix OAuth2 Client Auto-Configuration (S)

**Steps:**
1. Add `OAuth2ClientAutoConfiguration` to exclude list in `application.yml`
2. Remove unused login/consent templates and Thymeleaf dependency (SSO is deferred)
3. Remove `spring-boot-starter-oauth2-client` dependency from build.gradle.kts

**Files to modify:**
- `server/app/src/main/resources/application.yml`
- `server/app/src/main/resources/templates/login.html` (delete)
- `server/app/src/main/resources/templates/consent.html` (delete)
- `server/app/build.gradle.kts` (remove oauth2-client and Thymeleaf dependencies)

**Verification:**
```bash
# Build and run server without GitHub credentials
./gradlew :server:app:build
./gradlew :server:app:bootRun

# Verify no "Client id of registration 'github'" error
# Verify health endpoint responds: curl http://localhost:8080/actuator/health
```

### Phase 2: Fix Security Disabled Mode JWT Parsing (S)

**Steps:**
1. Update `securityFilterChainPermitAll()` to add JWT resource server configuration
2. Add `UserAuthenticationFilter` to the filter chain (after `BearerTokenAuthenticationFilter`)
3. Handle case where JWT is not present (filter should gracefully skip)

**Files to modify:**
- `server/app/src/main/kotlin/ai/flovyn/config/SecurityConfig.kt`

**Verification:**
```bash
# Run existing server tests
./gradlew :server:app:test

# Test with security disabled but JWT provided
# Start server with FLOVYN_SECURITY_ENABLED=false
# Send request with JWT - should extract userId
# Send request without JWT - should still work (userId null/default)
```

### Phase 3: Add Test Mode JWT Decoder (M)

**Steps:**
1. Add configuration property `flovyn.security.jwt.skip-signature-verification`
2. Create `UnsafeJwtDecoder` that parses JWT without signature verification
3. Add `@Profile("!prod")` guard to prevent accidental production use
4. Document the test mode configuration

**Files to modify:**
- `server/app/src/main/kotlin/ai/flovyn/config/SecurityConfig.kt` (or new `JwtConfig.kt`)
- `server/app/src/main/resources/application.yml`

**Verification:**
```bash
# Run all server tests
./gradlew :server:app:test

# Test with skip-signature-verification enabled
# Start server with:
#   FLOVYN_SECURITY_ENABLED=true
#   FLOVYN_SECURITY_JWT_SKIP_SIGNATURE_VERIFICATION=true
# Send request with self-signed JWT - should succeed

# Test with skip-signature-verification disabled (default)
# Send request with self-signed JWT - should fail with 401
```

### Phase 4: End-to-End Verification

**Steps:**
1. Build Docker image with all fixes
2. Run Rust SDK E2E tests against the container
3. Verify tenant creation and worker token APIs work

**Verification:**
```bash
# Build server Docker image
docker build -t flovyn-server-test:latest server/app/

# Run Rust SDK E2E tests
cd sdk-rust
cargo test --test e2e test_harness_setup -- --ignored --nocapture
```

---

## Current Test Setup

The E2E test harness (`sdk/tests/e2e/harness.rs`) currently:
1. Uses existing dev PostgreSQL (port 5435) and NATS (port 4222)
2. Starts Flovyn server container with `flovyn-server-test:latest` image
3. Waits for health check at `/actuator/health`
4. Attempts to create tenant and worker token via REST API
5. Uses generated JWT for authentication

**Test Command:**
```bash
cargo test --test e2e test_harness_setup -- --ignored --nocapture
```

---

## Test JWT Structure

The test generates JWTs with these claims (matching BetterAuth format):
```json
{
  "sub": "test-user-XXXXXXXX",
  "id": "test-user-XXXXXXXX",
  "name": "E2E Test User",
  "email": "e2e-test@example.com",
  "iss": "http://localhost:3000",
  "aud": "flovyn-server",
  "exp": 1765639090,
  "iat": 1765635490
}
```

Signed with RS256 algorithm using a pre-generated 2048-bit RSA key.

---

## TODO

- [x] Phase 1.1: Add OAuth2ClientAutoConfiguration to exclude list in application.yml (not needed - removed dependency)
- [x] Phase 1.2: Delete unused login.html and consent.html templates
- [x] Phase 1.3: Remove spring-boot-starter-oauth2-client dependency (unused)
- [x] Phase 1.4: Remove Thymeleaf dependencies if only used for SSO
- [x] Phase 2: Update securityFilterChainPermitAll to parse JWTs
- [x] Phase 3: Implement UnsafeJwtDecoder for test mode
- [x] Verify E2E tests pass with all fixes applied
- [x] Update Rust SDK test harness to use new configuration

## Changes Made

### Files Modified
1. **server/app/build.gradle.kts**
   - Removed `spring-boot-starter-oauth2-client` dependency
   - Removed `spring-boot-starter-thymeleaf` and `thymeleaf-extras-springsecurity6` dependencies

2. **server/app/src/main/kotlin/ai/flovyn/config/SecurityConfig.kt**
   - Added `JwtDecoder` injection via `ObjectProvider`
   - Updated `securityFilterChainWithAuth` to use injected decoder if available
   - Updated `securityFilterChainPermitAll` to optionally parse JWTs when a decoder is available
   - Added all filters (WorkerTokenAuthenticationFilter, UserAuthenticationFilter, TenantResolutionFilter) to permit-all chain

3. **server/app/src/main/kotlin/ai/flovyn/config/JwtConfiguration.kt** (new)
   - Added `unsafeJwtDecoder()` bean that skips signature verification
   - Activated by `flovyn.security.jwt.skip-signature-verification=true`
   - Parses JWT claims without cryptographic validation

### Files Deleted
- `server/app/src/main/resources/templates/login.html`
- `server/app/src/main/resources/templates/consent.html`

## E2E Test Configuration

To run the Rust SDK E2E tests against the server, use these environment variables:

```bash
# Enable security with signature verification skipped (for self-signed JWTs)
FLOVYN_SECURITY_ENABLED=true
FLOVYN_SECURITY_JWT_SKIP_SIGNATURE_VERIFICATION=true
```

This configuration:
1. Enables the full security filter chain (authentication required for protected endpoints)
2. Uses the `unsafeJwtDecoder` which parses JWT claims without signature validation
3. Allows E2E tests to generate self-signed JWTs that the server will accept