const footerLinks = [
  {
    label: "Terms",
    href: "https://docs.github.com/site-policy/github-terms/github-terms-of-service"
  },
  {
    label: "Privacy",
    href: "https://docs.github.com/site-policy/privacy-policies/github-privacy-statement"
  },
  {
    label: "Security",
    href: "https://github.com/security"
  },
  {
    label: "Docs",
    href: "https://docs.github.com/"
  },
  {
    label: "Contact",
    href: "https://support.github.com/?tags=dotcom-footer"
  }
];

export function SiteFooter() {
  return (
    <footer className="siteFooter">
      <a className="footerBrand" href="/" aria-label="GitMesh home">
        <img src="/gitmesh-logo-white.png" alt="" />
      </a>
      <span>© 2026 GitMesh, Inc.</span>
      <nav aria-label="Footer">
        {footerLinks.map((link) => (
          <a href={link.href} key={link.label}>
            {link.label}
          </a>
        ))}
      </nav>
    </footer>
  );
}
