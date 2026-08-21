export default function LoginPage() {
  return (
    <main className="loginPage">
      <a className="loginMark" href="/" aria-label="GitMesh home">
        <img src="/gitmesh-logo-white.png" alt="" />
      </a>

      <h1>Sign in to GitMesh</h1>

      <section className="loginPanel" aria-label="Sign in">
        <label>
          Username or email address
          <input autoComplete="username" name="login" type="text" />
        </label>

        <label>
          <span>
            Password
            <a href="/login">Forgot password?</a>
          </span>
          <input autoComplete="current-password" name="password" type="password" />
        </label>

        <a className="loginSubmit" href="/repo">
          Sign in
        </a>

        <button className="passkeyButton" type="button">
          Sign in with a passkey
        </button>
      </section>

      <section className="loginCreate">
        New to GitMesh? <a href="/login">Create an account</a>
      </section>

      <nav className="loginFooterLinks" aria-label="Login footer">
        <a href="/login">Terms</a>
        <a href="/login">Privacy</a>
        <a href="/login">Docs</a>
        <a href="/login">Contact GitMesh Support</a>
      </nav>
    </main>
  );
}
