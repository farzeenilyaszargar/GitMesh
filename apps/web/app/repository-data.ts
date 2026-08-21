import {
  CircleDot,
  Code2,
  File,
  FileCode2,
  Folder,
  GitPullRequest,
} from "lucide-react";

export const repository = {
  owner: "farzeen",
  name: "GitMesh",
  visibility: "Private",
  description:
    "Decentralized Git hosting with signed refs, encrypted private repositories, erasure-coded storage, and rebuildable gateways.",
  defaultBranch: "main",
  watchers: "128",
  forks: "312",
  stars: "2.4k",
  branches: 34,
  tags: 12,
  commits: 18,
  webTrustMode: {
    mode: "Opaque private",
    gateway: "reconstructs ciphertext",
    browser: "decrypts with WASM",
    keyLocation: "authorized device only",
    cachePolicy: "ciphertext cacheable"
  },
  latestCommit: {
    author: "fizzy",
    message: "implemented local daemon IPC and remote-helper bridge",
    hash: "5f22a81"
  }
};

export const repositories = [
  {
    name: "GitMesh",
    owner: "farzeen",
    visibility: "Private",
    description:
      "Decentralized Git hosting with signed refs, encrypted storage, and rebuildable gateways.",
    language: "Rust",
    stars: "2.4k",
    updated: "updated 2m ago",
    href: "/repo"
  },
  {
    name: "gitmesh-web",
    owner: "farzeen",
    visibility: "Private",
    description:
      "Next.js gateway interface for browsing GitMesh repositories and collaboration state.",
    language: "TypeScript",
    stars: "312",
    updated: "updated 14m ago",
    href: "/repo"
  },
  {
    name: "gitmesh-protocol",
    owner: "farzeen",
    visibility: "Public",
    description:
      "Protocol notes, deterministic schema sketches, and test vector planning.",
    language: "Markdown",
    stars: "128",
    updated: "updated 1h ago",
    href: "/repo"
  }
];

export const files = [
  {
    icon: Folder,
    name: "apps",
    message: "shape repository UI around GitMesh gateway data",
    time: "2m"
  },
  {
    icon: Folder,
    name: "crates",
    message: "add daemon socket and remote-helper boundaries",
    time: "14m"
  },
  {
    icon: Folder,
    name: "docs",
    message: "document protocol, storage, coordination, and web design",
    time: "1h"
  },
  {
    icon: Folder,
    name: "protocol",
    message: "stage deterministic object schema vectors",
    time: "1h"
  },
  {
    icon: FileCode2,
    name: "Cargo.toml",
    message: "wire GitMesh Rust workspace",
    time: "14m"
  },
  {
    icon: File,
    name: "README.md",
    message: "introduce decentralized Git hosting surface",
    time: "soon"
  }
];

export const tabs = [
  { key: "Code", label: "Code", badge: "", href: "/repo", icon: Code2 },
  { key: "Issues", label: "Issues", badge: "42", href: "/repo/issues", icon: CircleDot },
  {
    key: "Pull requests",
    label: "Pull requests",
    badge: "9",
    href: "/repo/pulls",
    icon: GitPullRequest
  }
] as const;

export const activity = [
  "V0 storage proof passed after six node losses",
  "Remote-helper handshake added",
  "Daemon socket accepted local proof request"
];

export const issues = [
  {
    id: 1,
    title: "Persist collaboration event logs",
    labels: ["protocol", "collaboration"],
    author: "farzeen",
    time: "opened 12m ago",
    status: "Open",
    comments: 4
  },
  {
    id: 2,
    title: "Add private repository key epoch UI",
    labels: ["security", "web"],
    author: "mesh-dev",
    time: "opened 18m ago",
    status: "Open",
    comments: 2
  },
  {
    id: 3,
    title: "Record durability receipts before ref publication",
    labels: ["storage", "correctness"],
    author: "fizzy",
    time: "opened 1h ago",
    status: "Triaged",
    comments: 7
  }
];

export const pullRequests = [
  {
    id: 1,
    title: "Wire gm collaboration commands",
    source: "collaboration-cli",
    target: "main",
    checks: "2 pending",
    author: "farzeen",
    reviews: 1,
    comments: 3
  },
  {
    id: 2,
    title: "Sketch trusted gateway mode checks",
    source: "private-gateway-mode",
    target: "main",
    checks: "review required",
    author: "mesh-dev",
    reviews: 2,
    comments: 5
  }
];

export const checks = [
  {
    name: "cargo test",
    state: "passing",
    detail: "44 protocol and integration checks"
  },
  {
    name: "cargo clippy",
    state: "passing",
    detail: "warnings denied across workspace"
  },
  {
    name: "next build",
    state: "passing",
    detail: "repository UI compiled"
  }
];
