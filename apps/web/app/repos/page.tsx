import { BookOpen, Search, Star } from "lucide-react";

import { SiteHeader } from "../repo-chrome";
import { repositories } from "../repository-data";

export default function RepositoriesPage() {
  return (
    <main>
      <SiteHeader />

      <section className="singleColumn">
        <div className="listToolbar">
          <label className="filterInput">
            <Search size={16} />
            <input
              aria-label="Find a repository"
              placeholder="Find a repository..."
            />
          </label>
          <button className="greenButton">New repository</button>
        </div>

        <section className="listPanel" aria-label="Repositories">
          <div className="listHeader">
            <div>
              <BookOpen size={16} />
              <strong>{repositories.length} repositories</strong>
            </div>
            <nav aria-label="Repository list filters">
              <a href="#">Type</a>
              <a href="#">Language</a>
              <a href="#">Sort</a>
            </nav>
          </div>

          {repositories.map((repo) => (
            <a className="listRow repoListRow" href={repo.href} key={repo.name}>
              <BookOpen size={18} />
              <div>
                <h2>
                  {repo.owner}/{repo.name}
                </h2>
                <p>{repo.description}</p>
                <div className="branchPair">
                  <span>{repo.language}</span>
                  <span>{repo.visibility}</span>
                  <span>{repo.updated}</span>
                </div>
              </div>
              <div className="rowMeta">
                <span>
                  <Star size={14} />
                  {repo.stars}
                </span>
              </div>
            </a>
          ))}
        </section>
      </section>
    </main>
  );
}
