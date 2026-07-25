type EthereumProvider = {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
};

type LoginChallenge = {
  challenge_id: string;
  message: string;
};

type LoginResult = {
  redirect?: string;
};

function ethereumProvider(): EthereumProvider {
  const provider = (window as Window & { ethereum?: EthereumProvider }).ethereum;
  if (!provider) {
    throw new Error("No Ethereum wallet found.");
  }
  return provider;
}

async function login() {
  const provider = ethereumProvider();
  const accounts = await provider.request({ method: "eth_requestAccounts" });
  const address = Array.isArray(accounts) ? accounts[0] : undefined;
  if (typeof address !== "string") {
    throw new Error("No Ethereum wallet account selected.");
  }

  const challengeResponse = await fetch("/auth/challenge", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ wallet_address: address }),
  });
  if (!challengeResponse.ok) {
    throw new Error(await challengeResponse.text());
  }
  const challenge = (await challengeResponse.json()) as LoginChallenge;
  const signature = await provider.request({
    method: "personal_sign",
    params: [challenge.message, address],
  });

  const verifyResponse = await fetch("/auth/verify", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      challenge_id: challenge.challenge_id,
      message: challenge.message,
      signature,
    }),
  });
  if (!verifyResponse.ok) {
    throw new Error(await verifyResponse.text());
  }
  const result = (await verifyResponse.json().catch(() => ({}))) as LoginResult;
  window.location.assign(result.redirect ?? "/account");
}

const button = document.querySelector<HTMLButtonElement>("#wallet-login");
const error = document.querySelector<HTMLElement>("#login-error");

button?.addEventListener("click", () => {
  void login().catch((reason: unknown) => {
    if (error) {
      error.textContent = reason instanceof Error ? reason.message : "Login failed.";
    }
  });
});
