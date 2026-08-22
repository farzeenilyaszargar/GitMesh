import {
  CheckCircle2,
  GitMerge,
  GitPullRequest,
  MessageSquare,
  Plus,
  Search,
  SlidersHorizontal
} from "lucide-react";

import { gatewayPullRequests } from "../api/gitmesh/backend";
import { RepoHeader, SiteHeader } from "../repo-chrome";
import { pullRequests as fallbackPullRequests } from "../repository-data";

export const dynamic = "force-dynamic";

export default async function PullRequestsPage() {
  const daemonPullRequests = await gatewayPullRequests();
  const pullRequests =
    daemonPullRequests.length > 0 ? daemonPullRequests : fallbackPullRequests;

  return (
    <main>
      <SiteHeader showRepositoryPath />
      <RepoHeader active="Pull requests" />

      <section className="singleColumn">
        <div className="listToolbar">
          <label className="filterInput">
            <Search size={16} />
            <input
              aria-label="Filter pull requests"
              defaultValue="is:pr is:open"
            />
          </label>
          <button>
            <SlidersHorizontal size={15} />
            Filters
          </button>
          <button className="greenButton">
            <Plus size={15} />
            New pull request
          </button>
        </div>

        <section className="listPanel" aria-label="Pull requests">
          <div className="listHeader">
            <div>
              <GitPullRequest size={16} />
              <strong>{pullRequests.length} Open</strong>
              <span>4 Merged</span>
            </div>
            <nav aria-label="Pull request list filters">
              <a href="#">Author</a>
              <a href="#">Reviews</a>
              <a href="#">Checks</a>
              <a href="#">Base</a>
            </nav>
          </div>

          {pullRequests.map((pullRequest) => (
            <a className="listRow" href="#" key={pullRequest.id}>
              <GitPullRequest size={18} />
              <div>
                <h2>{pullRequest.title}</h2>
                <p>
                  #{pullRequest.id} opened by {pullRequest.author};{" "}
                  {pullRequest.source} into {pullRequest.target}
                </p>
                <div className="branchPair">
                  <span>{pullRequest.source}</span>
                  <GitMerge size={14} />
                  <span>{pullRequest.target}</span>
                </div>
              </div>
              <div className="rowMeta">
                <span className="statePill mutedState">{pullRequest.checks}</span>
                <span>
                  <MessageSquare size={14} />
                  {pullRequest.comments}
                </span>
              </div>
            </a>
          ))}
        </section>

        <div className="protocolNote">
          <CheckCircle2 size={16} />
          <span>
            Pull requests use the same collaboration-event model, while merge
            visibility still depends on repository-specific ref coordination.
          </span>
        </div>
      </section>
    </main>
  );
}
