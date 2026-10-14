export function generatedReferenceDirectory(name) {
  return ['.git', 'target', 'node_modules', '.devenv', '.direnv', '.delta'].includes(name);
}

export function validateGithubPin(name, node, yaml) {
  const errors = [];
  if (node?.locked?.type !== 'github') return [`Nix input ${name} has no reviewed GitHub pin`];
  const { owner, repo, rev, narHash } = node.locked;
  if (!/^[a-f0-9]{40}$/.test(rev) || !/^sha256-[A-Za-z0-9+/]{43}=$/.test(narHash ?? '') || node.original?.rev !== rev) {
    errors.push(`Nix input ${name} has a moving original or lacks content identity`);
  }
  if (name !== 'devenv') {
    const lines = yaml.split('\n');
    const index = lines.indexOf(`  ${name}:`);
    if (index < 0 || lines[index + 1] !== `    url: github:${owner}/${repo}/${rev}`) {
      errors.push(`devenv.yaml does not use ${name}'s locked commit`);
    }
  }
  return errors;
}

export function referenceIdentity(url) {
  const commit = [
    /^https:\/\/github\.com\/[^/]+\/[^/]+\/(?:blob|tree|raw|archive)\/([a-f0-9]{40})(?:\/|$|#|\?|\.tar\.|\.zip)/,
    /^https:\/\/raw\.githubusercontent\.com\/[^/]+\/[^/]+\/([a-f0-9]{40})(?:\/|$)/,
    /^https:\/\/codeload\.github\.com\/[^/]+\/[^/]+\/(?:tar\.gz|zip)\/([a-f0-9]{40})(?:\/|$)/,
    /^github:[^/]+\/[^/]+\/([a-f0-9]{40})(?:\/|$|\?)/,
  ].map(pattern=>url.match(pattern)?.[1]).find(Boolean);
  const moving = /(?:\/(?:main|master|HEAD|latest|stable|rolling)(?:\/|\.|$)|:latest(?:$|\/))/.test(url);
  return { identity: commit ? { git_commit: commit } : null, status: commit ? 'commit-pinned' : moving ? 'moving-reference' : 'requires-classification' };
}
