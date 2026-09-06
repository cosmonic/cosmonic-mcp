# NATS security reference (NKey, JWT/operator, permissions, TLS, auth callout)

> Adapted from [kaustavdm/nats-skill](https://github.com/kaustavdm/nats-skill) (`security.md`, MIT — see `../LICENSE-MIT`). NATS-general reference: the code snippets are `nats.go`, which on Cosmonic Desktop is the **operator and tooling** side (provisioning, the `nats` CLI, server config), never the guest — a component uses the `wasmcloud:nats` WIT surface in `wasmcloud-nats.md` instead.

Source: https://docs.nats.io/nats-concepts/security

## Authentication Methods Overview

| Method | Config complexity | Security level | Notes |
|--------|------------------|----------------|-------|
| Token | Low | Low | Single shared secret |
| Username/Password | Low | Medium | bcrypt passwords recommended |
| NKey | Medium | High | Ed25519; no password over wire |
| JWT + NKey | High | High | Decentralized; no server restart for new users |
| TLS client cert | Medium | High | Cert CN/SAN maps to user |
| Auth callout | High | Configurable | Delegate to external service |

## NKey Basics

NKeys are Ed25519 keypairs. Identity is the public key; authentication is a cryptographic challenge-response (private key never leaves the client).

```bash
# Install nsc and nk tools
go install github.com/nats-io/nsc/v2@latest
go install github.com/nats-io/nkeys/nk@latest

# Generate an NKey pair
nk -gen user -pubout
# Outputs: SUABC...  (seed/private - keep secret)
#          UABC123... (public key - put in server config)
```

```go
// Go client with NKey seed file
opt, err := nats.NkeyOptionFromSeed("user.nk")
nc, err := nats.Connect("nats://localhost:4222", opt)

// Or with NKey seed string
nc, err := nats.Connect("nats://localhost:4222",
    nats.Nkey(publicKey, func(nonce []byte) ([]byte, error) {
        return privKey.Sign(nonce)
    }),
)
```

## JWT + Operator Model (Decentralized Auth)

Three-tier hierarchy:
1. **Operator** — controls server; issues account JWTs
2. **Account** — isolated namespace; issues user JWTs
3. **User** — application identity; has permissions

```
Operator (manages NATS servers)
└── Account A (e.g., "team-payments")
    ├── User app-service
    └── User worker
└── Account B (e.g., "team-orders")
    └── User api-gateway
```

Key property: Account admins can create/revoke users **without touching server config**.

```bash
# Setup with nsc
nsc add operator myoperator
nsc add account payments
nsc add user app-service --account payments
nsc describe user app-service --account payments

# Export user credentials file (JWT + NKey)
nsc generate creds -a payments -u app-service -o app-service.creds

# Push account JWTs to server resolver
nsc push --all
```

```go
// Connect with credentials file
nc, err := nats.Connect("nats://localhost:4222",
    nats.UserCredentials("app-service.creds"),
)
```

## Subject-Level Permissions

Users can have allow/deny lists for publish and subscribe separately:

```conf
users: [
  {
    user: publisher
    password: secret
    permissions: {
      publish: {
        allow: ["events.>", "commands.>"]
        deny:  ["commands.admin.>"]
      }
      subscribe: {
        allow: ["_INBOX.>"]    # only their own inbox (for request/reply)
        deny:  [">"]
      }
    }
  }
]
```

Note: Deny takes precedence over allow. If you want request/reply, always allow `_INBOX.>` in subscribe.

## Accounts and Multi-Tenancy

```conf
accounts {
  PAYMENTS: {
    jetstream: enabled
    users: [{ user: pay-svc, password: secret }]
    exports: [
      # Export a service (request/reply) to other accounts
      { service: "payment.process", accounts: [ORDERS] }
      # Export a stream (pub/sub) to other accounts
      { stream: "payment.events.>", accounts: [ORDERS] }
    ]
  }
  ORDERS: {
    jetstream: enabled
    users: [{ user: order-svc, password: secret }]
    imports: [
      { service: { account: PAYMENTS, subject: "payment.process" } }
      { stream:  { account: PAYMENTS, subject: "payment.events.>" } }
    ]
  }
}
```

## TLS Configuration

```conf
tls {
  cert_file: "/etc/nats/server-cert.pem"
  key_file:  "/etc/nats/server-key.pem"
  ca_file:   "/etc/nats/ca.pem"       # for mutual TLS
  verify:    true                      # require client certs
  timeout:   2                         # TLS handshake timeout
  cipher_suites: [
    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384"
    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384"
  ]
  min_version: 1.2
}
```

```go
// Go client with custom TLS
tlsConfig := &tls.Config{
    RootCAs:      certPool,
    Certificates: []tls.Certificate{clientCert},
}
nc, err := nats.Connect("tls://localhost:4222",
    nats.Secure(tlsConfig),
)
```

## Auth Callout (ADR-26)

Delegate authentication to an external service over NATS itself.

```conf
authorization {
  auth_callout {
    issuer: "AABC123..."          # account NKey that issues user JWTs
    auth_users: [callout-svc]     # users allowed to respond to auth requests
    account: AUTH_ACCOUNT
  }
}
```

The callout service subscribes to `$SYS.REQ.USER.AUTH` and responds with a signed user JWT or error.

## JetStream Encryption at Rest

```conf
jetstream {
  store_dir: "/data/nats"
  cipher: chacha      # ChaCha20-Poly1305 (faster on ARM/no-AES-NI hardware)
  # cipher: aes       # AES-256-GCM (faster on x86 with AES-NI)
  key: "$JS_ENCRYPT_KEY"  # from environment variable
}
```

The encryption key is never stored; if lost, data is unrecoverable.
