"use client";

import { useEffect, useRef, useState } from "react";
import {
  Bell,
  ChevronDown,
  GitFork,
  Plus,
  Search,
  Star
} from "lucide-react";

import { repository, tabs } from "./repository-data";

type RepoChromeProps = {
  active: (typeof tabs)[number]["key"];
};

type SiteHeaderProps = {
  showRepositoryPath?: boolean;
  profilePath?: string;
};

export function SiteHeader({ showRepositoryPath = false, profilePath }: SiteHeaderProps) {
  const [isProfileOpen, setIsProfileOpen] = useState(false);
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const profileMenuRef = useRef<HTMLDetailsElement>(null);
  const createMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (!profileMenuRef.current?.contains(event.target as Node)) {
        setIsProfileOpen(false);
      }

      if (!createMenuRef.current?.contains(event.target as Node)) {
        setIsCreateOpen(false);
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setIsProfileOpen(false);
        setIsCreateOpen(false);
      }
    }

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  return (
    <header className="siteHeader">
      <div className="headerLeft">
        <a className="brand" href="/" aria-label="GitMesh home">
          <img className="brandMark" src="/gitmesh-logo-white.png" alt="" />
        </a>
        {showRepositoryPath ? (
          <div className="headerRepoPath" aria-label="Current repository">
            <a href="/profile">{repository.owner}</a>
            <span>/</span>
            <a href="/repo">{repository.name}</a>
          </div>
        ) : null}
        {profilePath ? (
          <div className="headerProfilePath" aria-label="Current profile">
            <a href="/profile">{profilePath}</a>
          </div>
        ) : null}
      </div>

      <div className="headerCenter">
        <label className="searchBox">
          <Search size={16} />
          <input aria-label="Search GitMesh" placeholder="Search or jump to..." />
          <span>/</span>
        </label>
      </div>

      <div className="headerActions">
        <div className="createMenu" ref={createMenuRef}>
          <button
            className="createButton"
            aria-expanded={isCreateOpen}
            aria-label="Create new"
            onClick={() => setIsCreateOpen((current) => !current)}
          >
            <Plus size={16} />
            <ChevronDown size={14} />
          </button>
          <div className={`createPopover${isCreateOpen ? " open" : ""}`}>
            <a href="/repo/issues">New issue</a>
            <a href="/repos">New repository</a>
            <a href="/repo/pulls">New pull request</a>
            <a href="/repo">New organization</a>
          </div>
        </div>
        <details className="profileMenu" open={isProfileOpen} ref={profileMenuRef}>
          <summary
            className="avatarButton"
            aria-label="Open user navigation menu"
            title="Open user navigation menu"
            onClick={(event) => {
              event.preventDefault();
              setIsProfileOpen((current) => !current);
            }}
          >
            <img src="/prof_pic.jpeg" alt="fizzy profile picture" />
          </summary>
          <div className="profilePopover">
            <div className="profilePopoverIdentity">
              <img className="miniAvatar" src="/prof_pic.jpeg" alt="fizzy profile picture" />
              <div>
                <strong>fizzy</strong>
                <span>farzeen</span>
              </div>
            </div>
            <a href="/profile">Your profile</a>
            <a href="/profile?tab=repositories">Your repositories</a>
            <a href="/profile?tab=stars">Your stars</a>
            <a href="/issues">Your issues</a>
            <a href="/pulls">Your pull requests</a>
          </div>
        </details>
      </div>
    </header>
  );
}

export function RepoHeader({ active }: RepoChromeProps) {
  const [selectedActions, setSelectedActions] = useState({
    watch: false,
    fork: false,
    star: false
  });

  function toggleAction(action: keyof typeof selectedActions) {
    setSelectedActions((current) => ({
      ...current,
      [action]: !current[action]
    }));
  }

  return (
    <section className="repoHeader">
      <div className="repoHeaderInner">
        <div className="repoIdentity">
          <h1>{repository.name}</h1>
          <span className="pill mutedPill">{repository.visibility}</span>
        </div>

        <div className="repoActions">
          <button
            className={`repoActionButton${selectedActions.watch ? " selected" : ""}`}
            type="button"
            aria-pressed={selectedActions.watch}
            onClick={() => toggleAction("watch")}
          >
            <Bell size={14} />
            Watch
            <span>{repository.watchers}</span>
          </button>
          <button
            className={`repoActionButton${selectedActions.fork ? " selected" : ""}`}
            type="button"
            aria-pressed={selectedActions.fork}
            onClick={() => toggleAction("fork")}
          >
            <GitFork size={14} />
            Fork
            <span>{repository.forks}</span>
          </button>
          <button
            className={`repoActionButton${selectedActions.star ? " selected" : ""}`}
            type="button"
            aria-pressed={selectedActions.star}
            onClick={() => toggleAction("star")}
          >
            <Star size={14} />
            Star
            <span>{repository.stars}</span>
          </button>
        </div>
      </div>

      <nav className="repoTabs" aria-label="Repository sections">
        {tabs.map(({ key, label, badge, href, icon: Icon }) => (
          <a className={key === active ? "active" : ""} href={href} key={key}>
            <Icon size={16} />
            {label}
            {badge ? <span>{badge}</span> : null}
          </a>
        ))}
      </nav>
    </section>
  );
}
