import {
  BookOpen,
  CheckCircle2,
  ChevronDown,
  CircleDot,
  Code2,
  GitBranch,
  GitPullRequest,
  Globe2,
  History,
  MoreHorizontal,
  Network,
  Package,
  ShieldCheck,
  Tag,
  UploadCloud
} from "lucide-react";

import { RepoHeader, SiteHeader } from "./repo-chrome";
import type { GatewaySnapshot } from "./api/gitmesh/backend";
import {
  checks,
  files,
  issues,
  pullRequests,
  repository
} from "./repository-data";

type RepositoryViewProps = {
  snapshot?: GatewaySnapshot;
};

export default function RepositoryView({ snapshot }: RepositoryViewProps) {
  const daemonOnline = snapshot?.health.ok === true;
  const objectCount = snapshot?.repo.fields.objects ?? "0";
  const refCount = snapshot?.repo.fields.refs ?? "0";
  const checkpointCount = snapshot?.repo.fields.checkpoints ?? "0";
  const keyGrantCount = snapshot?.repo.fields.key_grants ?? snapshot?.keyGrants.fields.grants ?? "0";
  const latestEpoch = snapshot?.keyGrants.fields.latest_epoch ?? "none";

  return (
    <main>
      <SiteHeader showRepositoryPath />
      <RepoHeader active="Code" />

      <div className="workspace">
        <section className="contentColumn" aria-label="Repository files">
          <div className="branchToolbar">
            <button className="branchButton">
              <GitBranch size={15} />
              {repository.defaultBranch}
              <ChevronDown size={14} />
            </button>
            <div className="branchStats">
              <a href="#">
                <GitBranch size={14} />
                <strong>{repository.branches}</strong> branches
              </a>
              <a href="#">
                <Tag size={14} />
                <strong>{repository.tags}</strong> tags
              </a>
            </div>
            <div className="toolbarActions">
              <button>Go to file</button>
              <button>Add file</button>
              <button className="greenButton">
                <Code2 size={15} />
                Code
                <ChevronDown size={14} />
              </button>
            </div>
          </div>

          <section className="filePanel">
            <div className="commitBanner">
              <img className="miniAvatar" src="/prof_pic.jpeg" alt="fizzy profile picture" />
              <div className="commitText">
                <strong>{repository.latestCommit.author}</strong>
                <span> {repository.latestCommit.message}</span>
              </div>
              <a className="commitHash" href="#">
                {repository.latestCommit.hash}
              </a>
              <a className="commitCount" href="#">
                <History size={14} />
                {repository.commits} commits
              </a>
            </div>

            <div className="fileRows">
              {files.map(({ icon: Icon, name, message, time }) => (
                <a className="fileRow" href="#" key={name}>
                  <Icon size={17} />
                  <span className="fileName">{name}</span>
                  <span className="fileMessage">{message}</span>
                  <span className="fileTime">{time}</span>
                </a>
              ))}
            </div>
          </section>

          <div className="workPanels">
            <section className="workPanel">
              <div className="panelHeader">
                <CircleIcon />
                <strong>Issues</strong>
                <a href="/repo/issues">View all</a>
              </div>
              <div className="trackingRows">
                {issues.map((issue) => (
                  <a className="trackingRow" href="#" key={issue.id}>
                    <CircleDotIcon />
                    <div>
                      <strong>{issue.title}</strong>
                      <span>
                        #{issue.id} {issue.time} by {issue.author}
                      </span>
                    </div>
                    <span className="statePill">{issue.status}</span>
                    <div className="labelStack">
                      {issue.labels.map((label) => (
                        <span key={label}>{label}</span>
                      ))}
                    </div>
                  </a>
                ))}
              </div>
            </section>

            <section className="workPanel">
              <div className="panelHeader">
                <GitPullRequest size={16} />
                <strong>Pull requests</strong>
                <a href="/repo/pulls">View all</a>
              </div>
              <div className="trackingRows">
                {pullRequests.map((pullRequest) => (
                  <a className="trackingRow" href="#" key={pullRequest.id}>
                    <GitPullRequest size={17} />
                    <div>
                      <strong>{pullRequest.title}</strong>
                      <span>
                        #{pullRequest.id} {pullRequest.source} into{" "}
                        {pullRequest.target} by {pullRequest.author}
                      </span>
                    </div>
                    <span className="statePill mutedState">
                      {pullRequest.checks}
                    </span>
                  </a>
                ))}
              </div>
            </section>
          </div>

          <section className="readmePanel">
            <div className="panelHeader">
              <BookOpen size={16} />
              <strong>README</strong>
              <button className="iconButton subtle" aria-label="README options">
                <MoreHorizontal size={16} />
              </button>
            </div>
            <article className="readmeBody">
              <h2>GitMesh</h2>
              <p>
                A decentralized Git hosting platform where ordinary Git clients
                push and fetch through a local daemon while repository data
                lives on a verifiable storage network.
              </p>
              <div className="codeBlock">
                <code>git remote add origin gitmesh://farzeen/gitmesh</code>
                <code>git push origin main</code>
              </div>
              <h3>What is already running</h3>
              <ul>
                <li>Typed protocol CIDs and canonical object envelopes</li>
                <li>XChaCha20-Poly1305 segment encryption wrappers</li>
                <li>Erasure-coded V0 local storage proof</li>
                <li>In-memory availability directory records</li>
                <li>Git object ID compatibility primitives</li>
                <li>Local daemon socket and remote-helper handshake</li>
              </ul>
            </article>
          </section>
        </section>

        <aside className="sideColumn" aria-label="Repository metadata">
          <section className="sidePanel aboutPanel">
            <div className="sideTitleRow">
              <h2>About</h2>
              <button className="iconButton subtle" aria-label="Edit about">
                <MoreHorizontal size={16} />
              </button>
            </div>
            <p>
              {repository.description}
            </p>
            <div className="linkList">
              <a href="#">
                <Globe2 size={15} />
                gitmesh.dev
              </a>
              <a href="#">
                <BookOpen size={15} />
                Readme
              </a>
              <a href="#">
                <ShieldCheck size={15} />
                Security policy
              </a>
            </div>
            <div className="topics">
              <a href="#">git</a>
              <a href="#">rust</a>
              <a href="#">p2p</a>
              <a href="#">storage</a>
              <a href="#">cryptography</a>
            </div>
          </section>

          <section className="sidePanel">
            <h2>GitMesh network</h2>
            <div className="healthRows">
              <div>
                <Network size={16} />
                <span>Daemon</span>
                <strong>{daemonOnline ? "online" : "offline"}</strong>
              </div>
              <div>
                <CheckCircle2 size={16} />
                <span>Objects</span>
                <strong>{objectCount}</strong>
              </div>
              <div>
                <UploadCloud size={16} />
                <span>Refs</span>
                <strong>{refCount}</strong>
              </div>
              <div>
                <ShieldCheck size={16} />
                <span>Key grants</span>
                <strong>{keyGrantCount}</strong>
              </div>
              <div>
                <History size={16} />
                <span>Checkpoints</span>
                <strong>{checkpointCount}</strong>
              </div>
            </div>
            <p className="smallText">
              {daemonOnline
                ? `Live from gitmeshd. Latest key epoch: ${latestEpoch}.`
                : "Start gitmeshd to populate live repository state."}
            </p>
          </section>

          <section className="sidePanel">
            <h2>Web trust mode</h2>
            <div className="trustModeBox">
              <div>
                <ShieldCheck size={16} />
                <strong>{repository.webTrustMode.mode}</strong>
              </div>
              <dl>
                <div>
                  <dt>Gateway</dt>
                  <dd>{repository.webTrustMode.gateway}</dd>
                </div>
                <div>
                  <dt>Browser</dt>
                  <dd>{repository.webTrustMode.browser}</dd>
                </div>
                <div>
                  <dt>Repo key</dt>
                  <dd>{repository.webTrustMode.keyLocation}</dd>
                </div>
                <div>
                  <dt>Cache</dt>
                  <dd>{repository.webTrustMode.cachePolicy}</dd>
                </div>
              </dl>
            </div>
          </section>

          <section className="sidePanel">
            <h2>Checks</h2>
            <div className="checkRows">
              {checks.map((check) => (
                <div key={check.name}>
                  <CheckCircle2 size={16} />
                  <span>
                    <strong>{check.name}</strong>
                    {check.detail}
                  </span>
                  <em>{check.state}</em>
                </div>
              ))}
            </div>
          </section>

          <section className="sidePanel">
            <h2>Releases</h2>
            <a className="releaseLink" href="#">
              <Package size={16} />
              V0 storage proof
            </a>
            <p className="smallText">1 release published</p>
          </section>

          <section className="sidePanel">
            <h2>Packages</h2>
            <div className="packageRow">
              <Package size={16} />
              gitmesh-storage
            </div>
            <div className="packageRow">
              <Package size={16} />
              gitmeshd
            </div>
          </section>

          <section className="sidePanel">
            <h2>Contributors</h2>
            <div className="contributors">
              <span>F</span>
              <span>G</span>
              <span>R</span>
              <span>N</span>
            </div>
          </section>

          <section className="sidePanel">
            <h2>Languages</h2>
            <div className="languageBar">
              <span className="rust"></span>
              <span className="ts"></span>
              <span className="docs"></span>
            </div>
            <div className="languageLegend">
              <span>Rust 68%</span>
              <span>TypeScript 21%</span>
              <span>Docs 11%</span>
            </div>
          </section>
        </aside>
      </div>
    </main>
  );
}

function CircleIcon() {
  return <CircleDotIcon />;
}

function CircleDotIcon() {
  return <CircleDot size={16} />;
}
