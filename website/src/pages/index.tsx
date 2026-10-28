import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';

function ProductPreview() {
  return (
    <div className="product-preview">
      <img src="/img/dashboard.png" alt="HyperVibes operator dashboard" />
    </div>
  );
}

export default function Home() {
  return (
    <Layout
      title="AI trading agents for Hyperliquid"
      description="Build and supervise AI trading agents for Hyperliquid."
    >
      <main className="homepage">
        <section className="hero">
          <div className="hero-copy">
            <h1 className="hero-title">
              <img src="/img/hypervibes.png" alt="HyperVibes" width="2172" height="724" />
            </h1>
            <p className="hero-summary">Agentic Trading Harness</p>
            <div className="hero-actions">
              <a className="button button--primary" href="#quick-start">Self-Host</a>
              <Link className="button button--secondary" to="/docs/">Documentation</Link>
              <a className="button button--secondary" href="https://github.com/idft/hypervibes" target="_blank" rel="noreferrer">
                <img className="button-icon button-icon--image" src="/img/github.png" alt="" aria-hidden="true" /> GitHub
              </a>
              <a className="button button--secondary" href="https://discord.gg/Up39Qvqmkh" target="_blank" rel="noreferrer">
                <img className="button-icon button-icon--image" src="/img/discord.png" alt="" aria-hidden="true" /> Discord
              </a>
              <a className="button button--secondary" href="https://x.com/hypervibes.ai" target="_blank" rel="noreferrer">
                <img className="button-icon button-icon--image" src="/img/x.png" alt="" aria-hidden="true" /> X
              </a>
            </div>
          </div>
          <ProductPreview />
        </section>

        <section className="workflow-section">
          <div>
            <h2>An Agentic Trading Harness</h2>
          </div>
          <ol className="workflow-list">
            <li><span>01</span><div><strong>Batteries included</strong><p>Built-in tools for trading, research, technical analysis, and strategy development.</p></div></li>
            <li><span>02</span><div><strong>Agents that work together</strong><p>Create specialized agents to analyze markets, manage trades, and review results.</p></div></li>
            <li><span>03</span><div><strong>Built on Pi Durable</strong><p>Persistent agent sessions, scheduled work, and memory that survives restarts.</p></div></li>
            <li><span>04</span><div><strong>Your keys. Your coins.</strong><p>Self-hosted and Open Source. Connect your own wallet and AI providers. Stay in control of your trading account.</p></div></li>
            <li><span>05</span><div><strong>Trade on Hyperliquid</strong><p>Connect your agents directly to the Hyperliquid decentralized exchange.</p></div></li>
          </ol>
        </section>

        <section className="quick-start-section" id="quick-start">
          <div className="quick-start-copy">
            <p className="eyebrow">QUICK START</p>
              <h2>Self-host on your own server.</h2>
            <p>
              HyperVibes runs locally from source while packaged installs are
              in progress. Install Node.js and pnpm, then connect your wallet
              and provider from the operator interface.
            </p>
              <Link className="quick-start-link" to="/docs/quick-start">
                Get Started
                <svg aria-hidden="true" viewBox="0 0 24 24">
                  <path d="M5 12h14m-6-6 6 6-6 6" />
                </svg>
              </Link>
          </div>
          <div className="terminal" aria-label="Example HyperVibes development setup command">
            <div className="terminal-bar">
              <span><i /><i /><i /></span>
              <small>terminal</small>
            </div>
            <pre><code><span className="terminal-comment"># Clone the repository and install dependencies</span>{'\n'}<span className="terminal-prompt">$</span> git clone https://github.com/idft/hypervibes.git{'\n'}<span className="terminal-prompt">$</span> cd hypervibes &amp;&amp; pnpm install{'\n\n'}<span className="terminal-comment"># Build and start HyperVibes; no .env required</span>{'\n'}<span className="terminal-prompt">$</span> pnpm start</code></pre>
          </div>
        </section>

      </main>
    </Layout>
  );
}
