# ADR-002: libp2p Networking

## Status

Accepted for initial implementation.

## Context

GitMesh needs peer discovery, NAT traversal, relay support, authenticated
transports, and provider lookup without making any bootstrap server
authoritative.

## Decision

Use libp2p for V1 networking.

## PROTOCOL REQUIREMENT

Nodes must discover peers and providers through replaceable, non-authoritative
infrastructure and verify all application data cryptographically.

## IMPLEMENTATION CHOICE

Use libp2p QUIC, TCP fallback, Noise/TLS, Kademlia, AutoNAT, DCUtR, Circuit
Relay v2, and mDNS for local discovery.

## INITIAL DEFAULT

The DHT is limited to routing and provider discovery. Availability directories
hold signed, expiring storage records.

## FUTURE OPTION

Alternative discovery overlays or transports can be added if they preserve
signed provider records and do not become authoritative application databases.
