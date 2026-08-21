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
      description="Build and supervise OpenCode-backed trading agents for Hyperliquid."
    >
      <main className="homepage">
        <section className="hero">
          <div className="hero-copy">
            <div className="hero-title">
              <img src="/img/logo.png" alt="" aria-hidden="true" />
              <h1>HyperVibes</h1>
            </div>
            <p className="hero-summary">
              Open-source Agentic Trading Harness for{' '}
              <a className="hyperliquid-link" href="https://app.hyperliquid.xyz/trade" target="_blank" rel="noreferrer">
                <img src="/img/hyperliquid.png" alt="" aria-hidden="true" /> Hyperliquid
              </a>
            </p>
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
            <p className="eyebrow">FEATURES</p>
            <h2>Agents for the full trading loop.</h2>
          </div>
          <ol className="workflow-list">
            <li><span>01</span><div><strong>Run agents on candle close</strong><p>Schedule analysis exactly where your strategy starts: on the close of the candle that matters.</p></div></li>
            <li><span>02</span><div><strong>Agent self-review and improvement</strong><p>Give agents a durable memory of their decisions, outcomes, and next adjustments.</p></div></li>
            <li><span>03</span><div><strong>Custom strategy code generation</strong><p>Turn a trading idea into strategy code your agent can inspect and evolve.</p></div></li>
            <li><span>04</span><div><strong>Trade via chat</strong><p>Ask questions, review context, and take action without leaving the conversation.</p></div></li>
            <li><span>05</span><div><strong>Use any model</strong><p>Choose any model from any AI provider.</p></div></li>
          </ol>
        </section>

        <section className="quick-start-section" id="quick-start">
          <div className="quick-start-copy">
            <p className="eyebrow">QUICK START</p>
              <h2>Self-host on your own server.</h2>
            <p>
              HyperVibes is designed to run locally in a container stack. Pull
              the image, configure your environment, then connect your wallet
              and provider from the operator interface.
            </p>
              <Link className="quick-start-link" to="/docs/quick-start">
                Get Started
                <svg aria-hidden="true" viewBox="0 0 24 24">
                  <path d="M5 12h14m-6-6 6 6-6 6" />
                </svg>
              </Link>
          </div>
          <div className="terminal" aria-label="Example HyperVibes container setup command">
            <div className="terminal-bar">
              <span><i /><i /><i /></span>
              <small>terminal</small>
            </div>
            <pre><code><span className="terminal-comment"># Download the latest Compose file</span>{'\n'}<span className="terminal-prompt">$</span> umask 077 &amp;&amp; curl -fsSLo podman-compose.yaml https://raw.githubusercontent.com/idft/hypervibes/master/podman-compose.yaml{'\n\n'}<span className="terminal-comment"># Generate local secrets, then start the stack</span>{'\n'}<span className="terminal-prompt">$</span> KEY=$(openssl rand -hex 32) &amp;&amp; CTRL=$(openssl rand -hex 32) &amp;&amp; OC=$(openssl rand -hex 32) &amp;&amp; DB=$(openssl rand -hex 32) &amp;&amp; sed -i -e "s/REPLACE_KEY/$KEY/" -e "s/REPLACE_CTRL/$CTRL/" -e "s/REPLACE_OC/$OC/" -e "s/REPLACE_DB/$DB/" podman-compose.yaml{'\n'}<span className="terminal-prompt">$</span> podman-compose up -d</code></pre>
          </div>
        </section>

      </main>
    </Layout>
  );
}
