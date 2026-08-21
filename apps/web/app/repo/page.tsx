import { gatewaySnapshot } from "../api/gitmesh/backend";
import RepositoryView from "../repo-view";

export const dynamic = "force-dynamic";

export default async function RepoPage() {
  const snapshot = await gatewaySnapshot();
  return <RepositoryView snapshot={snapshot} />;
}
