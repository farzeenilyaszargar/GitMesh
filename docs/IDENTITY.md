# GitMesh Identity

GitMesh separates product authentication from cryptographic protocol identity.
Logging into the website is not the same as possessing authority to sign
repository protocol operations.

## Account Root Identity

```text
Account Root Identity
        |
        +-- MacBook device key
        +-- desktop device key
        +-- mobile device key
        +-- browser device key
```

Repository permissions reference stable account or organization identities, not
individual device keys. Device keys are independently revocable.

## Device Certificates

`DeviceCertificate` binds a device key to an account:

```text
DeviceCertificate {
  account_id
  device_id
  device_public_key
  capabilities
  issued_at
  expires_at
  parent_certificate
  signature_by_account_or_recovery_authority
}
```

Shorter-lived browser device certificates are preferred for web sessions.

## Revocation

`DeviceRevocation` invalidates future authority of a device. Clients must check
revocation state before accepting signatures for mutable operations. Revocation
does not erase data already downloaded by that device.

## Repository Membership

`MembershipGrant` and `MembershipRevocation` authorize accounts or organizations
for repository roles such as owner, maintainer, writer, reader, trusted compute,
or auditor. Private repositories additionally grant encrypted key material for
the relevant key epoch.

## Organizations

Organization identity is a root identity with delegated administrators and
recovery policy. Organization repositories are controlled by org policy, not by a
single user's device key.

## Account Recovery

Recovery must allow a user or organization to restore authority without GitMesh
holding plaintext root keys. Candidate mechanisms:

- threshold recovery contacts
- hardware-backed recovery devices
- encrypted recovery package protected by Argon2id
- organization admin quorum

Recovery events are signed, auditable, and able to rotate compromised keys.
