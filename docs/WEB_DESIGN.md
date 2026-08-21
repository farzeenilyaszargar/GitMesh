# GitMesh Web Design Direction

GitMesh's web product should feel familiar to developers who know GitHub while
remaining visually distinct and legally original.

## Product Architecture

Use GitHub-like information architecture:

- global navigation with search, pull requests, issues, explore, docs, and
  account actions
- repository header with owner/name, visibility, watch/star/fork actions
- repository tabs for code, issues, pull requests, actions, projects, security,
  insights, and settings
- code view with branch selector, latest commit strip, file browser, README, and
  repository metadata rail
- future organization, issue, pull request, action, and security pages should
  preserve these familiar workflow positions

## Visual Direction

The visual design should be darker and more premium than GitHub:

- near-black base surfaces with layered charcoal panels
- restrained borders and subtle depth
- cool accent colors such as mint and blue for verified network state
- compact, developer-focused density
- 8px-or-less corner radius for cards and panels
- no decorative blobs or one-note purple/blue gradients

## GitMesh-Specific Differentiation

Surface decentralized-state concepts in places where GitHub would show ordinary
repo metadata:

- latest signed checkpoint
- durable shard count
- repair queue
- gateway/cache status
- coordinator availability
- private repository trust mode

These should read as normal repository health metadata, not marketing copy.

## Implementation

The web application lives in `apps/web` as a Next app. The first implemented
surface is a repository page modeled on familiar GitHub repository architecture
with GitMesh-specific network health and a darker premium visual system.
