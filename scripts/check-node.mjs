import { pathToFileURL } from "node:url";

export const minimumNodeVersion = "22.23.3";

export function assertSupportedNode(version = process.versions.node) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  const minimum = minimumNodeVersion.split(".").map(Number);
  const actual = match?.slice(1).map(Number);
  const difference = actual?.findIndex((part, index) => part !== minimum[index]);
  if (!actual || (difference !== -1 && actual[difference] < minimum[difference])) {
    throw new Error(
      `HyperVibes requires Node.js >=${minimumNodeVersion}; found ${version}. Upgrade Node.js before building or starting HyperVibes.`,
    );
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    assertSupportedNode();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
