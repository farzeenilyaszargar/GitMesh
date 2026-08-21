import { ArrowRight } from "lucide-react";
import { BuilderCount } from "./builder-count";

export default function Home() {
  return (
    <main className="marketingPage">
      <header className="marketingHeader">
        <a className="marketingBrand" href="/" aria-label="GitMesh home">
          <img src="/gitmesh-logo-white.png" alt="" />
        </a>

        <nav className="marketingNav" aria-label="Main navigation">
          <a className="loginTextLink" href="/login">Log in</a>
          <a className="loginLink" href="/login">Sign up</a>
        </nav>
      </header>

      <section className="marketingHero">
        <div className="heroSignal" aria-hidden="true">
          <span />
          <span />
          <span />
        </div>
        <div className="heroGlass">
          <p className="heroEyebrow">
            <BuilderCount />
          </p>
          <h1>
            <span>Private Git hosting</span>
            <strong>built to stay online.</strong>
          </h1>
          <p>
            Secure P2P repos stay encrypted, verifiable, and yours. No platform
            owner can sell your private code, and no central outage can take it
            offline.
          </p>
          <div className="heroActions">
            <a className="signupButton" href="/login">
              Sign up now
              <ArrowRight size={18} />
            </a>
            <a className="secondaryHeroButton" href="/login">
              Log in
            </a>
          </div>
        </div>
      </section>
    </main>
  );
}
