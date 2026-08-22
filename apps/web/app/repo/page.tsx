import {
  gatewayIssues,
  gatewayPullRequests,
  gatewaySnapshot
} from "../api/gitmesh/backend";
import RepositoryView from "../repo-view";
import {
  issues as fallbackIssues,
  pullRequests as fallbackPullRequests
} from "../repository-data";

export const dynamic = "force-dynamic";

export default async function RepoPage() {
  const [snapshot, daemonIssues, daemonPullRequests] = await Promise.all([
    gatewaySnapshot(),
    gatewayIssues(),
    gatewayPullRequests()
  ]);
  return (
    <RepositoryView
      snapshot={snapshot}
      issues={daemonIssues.length > 0 ? daemonIssues : fallbackIssues}
      pullRequests={
        daemonPullRequests.length > 0
          ? daemonPullRequests
          : fallbackPullRequests
      }
    />
  );
}
