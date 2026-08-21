import {
  BookOpen,
  Building2,
  CalendarDays,
  Link as LinkIcon,
  MapPin,
  Search,
  Star,
  Users
} from "lucide-react";

import { SiteHeader } from "../repo-chrome";
import { repositories } from "../repository-data";

const contributionWeeks = Array.from({ length: 53 }, (_, week) =>
  Array.from({ length: 7 }, (_, day) => {
    const score = (week * 7 + day * 11 + (week % 6) * 3) % 19;
    if (score < 8) return 0;
    if (score < 12) return 1;
    if (score < 15) return 2;
    if (score < 18) return 3;
    return 4;
  })
);

const contributionMonths = [
  { label: "Jan", span: 5 },
  { label: "Feb", span: 4 },
  { label: "Mar", span: 5 },
  { label: "Apr", span: 4 },
  { label: "May", span: 5 },
  { label: "Jun", span: 4 },
  { label: "Jul", span: 5 },
  { label: "Aug", span: 4 },
  { label: "Sep", span: 5 },
  { label: "Oct", span: 4 },
  { label: "Nov", span: 4 },
  { label: "Dec", span: 4 }
];

type ProfilePageProps = {
  searchParams?: Promise<{
    tab?: string;
  }>;
};

const starredRepositories = repositories.slice(0, 2);

export default async function ProfilePage({ searchParams }: ProfilePageProps) {
  const params = await searchParams;
  const activeTab =
    params?.tab === "repositories" || params?.tab === "stars"
      ? params.tab
      : "overview";

  return (
    <main>
      <SiteHeader profilePath="farzeen" />

      <section className="profileShell">
        <aside className="profileSidebar">
          <img className="profileAvatar" src="/prof_pic.jpeg" alt="fizzy profile picture" />
          <h1>fizzy</h1>
          <p className="profileHandle">farzeen</p>
          <p className="profileBio">
            Building GitMesh, a verifiable Git collaboration network with a
            familiar developer experience.
          </p>
          <button className="profileEditButton">Edit profile</button>
          <div className="profileStats">
            <a href="#">
              <Users size={15} />
              <strong>128</strong> followers
            </a>
            <a href="#">
              <strong>34</strong> following
            </a>
          </div>
          <div className="profileFacts">
            <span>
              <Building2 size={15} />
              GitMesh Labs
            </span>
            <span>
              <MapPin size={15} />
              Local development
            </span>
            <a href="#">
              <LinkIcon size={15} />
              gitmesh.dev
            </a>
            <span>
              <CalendarDays size={15} />
              Joined August 2026
            </span>
          </div>
        </aside>

        <section className="profileMain">
          <nav className="profileTabs" aria-label="Profile sections">
            <a className={activeTab === "overview" ? "active" : ""} href="/profile">
              Overview
            </a>
            <a
              className={activeTab === "repositories" ? "active" : ""}
              href="/profile?tab=repositories"
            >
              Repositories
              <span>{repositories.length}</span>
            </a>
            <a className={activeTab === "stars" ? "active" : ""} href="/profile?tab=stars">
              Stars
              <span>{starredRepositories.length}</span>
            </a>
          </nav>

          {activeTab === "overview" ? <ProfileOverview /> : null}
          {activeTab === "repositories" ? (
            <ProfileRepositoryList
              countLabel={`${repositories.length} repositories`}
              emptyLabel="Find a repository..."
              repos={repositories}
            />
          ) : null}
          {activeTab === "stars" ? (
            <ProfileRepositoryList
              countLabel={`${starredRepositories.length} starred repositories`}
              emptyLabel="Find a starred repository..."
              repos={starredRepositories}
            />
          ) : null}
        </section>
      </section>
    </main>
  );
}

function ProfileOverview() {
  return (
    <>
      <section>
        <div className="profileSectionHeader">
          <h2>Pinned</h2>
          <a href="/profile?tab=repositories">Customize your pins</a>
        </div>
        <div className="pinnedGrid">
          {repositories.slice(0, 2).map((repo) => (
            <a className="pinnedRepo" href={repo.href} key={repo.name}>
              <div>
                <BookOpen size={16} />
                <strong>{repo.name}</strong>
                <span>{repo.visibility}</span>
              </div>
              <p>{repo.description}</p>
              <footer>
                <span>{repo.language}</span>
                <span>
                  <Star size={14} />
                  {repo.stars}
                </span>
              </footer>
            </a>
          ))}
        </div>
      </section>

      <section className="contributionPanel">
        <div className="profileSectionHeader">
          <h2>48 contributions this year</h2>
        </div>
        <div className="contributionCalendar" aria-label="Contribution activity">
          <div className="contributionMonths" aria-hidden="true">
            {contributionMonths.map((month) => (
              <span style={{ gridColumn: `span ${month.span}` }} key={month.label}>
                {month.label}
              </span>
            ))}
          </div>
          <div className="contributionBody">
            <div className="contributionWeekdays" aria-hidden="true">
              <span></span>
              <span>Mon</span>
              <span></span>
              <span>Wed</span>
              <span></span>
              <span>Fri</span>
              <span></span>
            </div>
            <div className="contributionGrid">
              {contributionWeeks.map((week, weekIndex) => (
                <div className="contributionWeek" key={`week-${weekIndex}`}>
                  {week.map((level, dayIndex) => (
                    <span
                      className={`contributionCell level${level}`}
                      key={`${weekIndex}-${dayIndex}`}
                      title={`${level} contributions`}
                    ></span>
                  ))}
                </div>
              ))}
            </div>
          </div>
          <div className="contributionLegend" aria-hidden="true">
            <span>Less</span>
            {[0, 1, 2, 3, 4].map((level) => (
              <span className={`contributionCell level${level}`} key={level}></span>
            ))}
            <span>More</span>
          </div>
        </div>
      </section>

      <section className="activityPanel">
        <h2>Contribution activity</h2>
        <div className="profileTimeline">
          <div>
            <BookOpen size={16} />
            <span>
              Created repository <a href="/repo">farzeen/GitMesh</a>
            </span>
            <em>Today</em>
          </div>
          <div>
            <BookOpen size={16} />
            <span>
              Opened issue <a href="/repo/issues">Persist collaboration event logs</a>
            </span>
            <em>Today</em>
          </div>
          <div>
            <BookOpen size={16} />
            <span>
              Opened pull request <a href="/repo/pulls">Wire gm collaboration commands</a>
            </span>
            <em>Today</em>
          </div>
        </div>
      </section>
    </>
  );
}

function ProfileRepositoryList({
  countLabel,
  emptyLabel,
  repos
}: {
  countLabel: string;
  emptyLabel: string;
  repos: typeof repositories;
}) {
  return (
    <section className="profileList">
      <div className="profileListToolbar">
        <label className="filterInput">
          <Search size={16} />
          <input aria-label={emptyLabel} placeholder={emptyLabel} />
        </label>
      </div>
      <div className="listPanel" aria-label={countLabel}>
        <div className="listHeader">
          <div>
            <BookOpen size={16} />
            <strong>{countLabel}</strong>
          </div>
        </div>
        {repos.map((repo) => (
          <a className="listRow repoListRow" href={repo.href} key={repo.name}>
            <BookOpen size={18} />
            <div>
              <h2>{repo.name}</h2>
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
      </div>
    </section>
  );
}
