import { CheckCircle2, CircleDot, MessageSquare, Plus, Search, SlidersHorizontal } from "lucide-react";

import { gatewayIssues } from "../api/gitmesh/backend";
import { RepoHeader, SiteHeader } from "../repo-chrome";
import { issues as fallbackIssues } from "../repository-data";

export const dynamic = "force-dynamic";

export default async function IssuesPage() {
  const daemonIssues = await gatewayIssues();
  const issues = daemonIssues.length > 0 ? daemonIssues : fallbackIssues;

  return (
    <main>
      <SiteHeader showRepositoryPath />
      <RepoHeader active="Issues" />

      <section className="singleColumn">
        <div className="listToolbar">
          <label className="filterInput">
            <Search size={16} />
            <input
              aria-label="Filter issues"
              defaultValue="is:issue is:open"
            />
          </label>
          <button>
            <SlidersHorizontal size={15} />
            Filters
          </button>
          <button className="greenButton">
            <Plus size={15} />
            New issue
          </button>
        </div>

        <section className="listPanel" aria-label="Issues">
          <div className="listHeader">
            <div>
              <CircleDot size={16} />
              <strong>{issues.length} Open</strong>
              <span>12 Closed</span>
            </div>
            <nav aria-label="Issue list filters">
              <a href="#">Author</a>
              <a href="#">Label</a>
              <a href="#">Milestones</a>
            </nav>
          </div>

          {issues.map((issue) => (
            <a className="listRow" href="#" key={issue.id}>
              <CircleDot size={18} />
              <div>
                <h2>{issue.title}</h2>
                <p>
                  #{issue.id} {issue.time} by {issue.author}
                </p>
                <div className="labelStack">
                  {issue.labels.map((label) => (
                    <span key={label}>{label}</span>
                  ))}
                </div>
              </div>
              <div className="rowMeta">
                <span className="statePill">{issue.status}</span>
                <span>
                  <MessageSquare size={14} />
                  {issue.comments}
                </span>
              </div>
            </a>
          ))}
        </section>

        <div className="protocolNote">
          <CheckCircle2 size={16} />
          <span>
            Issues are rendered from daemon collaboration-event summaries when
            available, with static data only as an offline fallback.
          </span>
        </div>
      </section>
    </main>
  );
}
