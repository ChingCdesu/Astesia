#!/usr/bin/env node

import { chmod, copyFile, mkdir } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { spawn, spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, '..');
const manifestPath = resolve(repositoryRoot, 'src-tauri', 'Cargo.toml');
const binariesDirectory = resolve(repositoryRoot, 'src-tauri', 'binaries');

function readHostTriple() {
  const result = spawnSync('rustc', ['--print', 'host-tuple'], {
    cwd: repositoryRoot,
    encoding: 'utf8',
  });

  if (result.error) {
    throw new Error(`Could not run rustc to determine the host target: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `rustc --print host-tuple failed with exit code ${result.status}: ${result.stderr.trim()}`,
    );
  }

  return result.stdout.trim();
}

function isEnabled(value) {
  return /^(1|true|yes|on)$/i.test(value?.trim() ?? '');
}

async function buildSidecar(targetTriple, profile) {
  const cargoArguments = [
    'build',
    '--manifest-path',
    manifestPath,
    '--target',
    targetTriple,
    '--profile',
    profile,
    '--bin',
    'astesia-mcp',
    '--message-format=json-render-diagnostics',
  ];

  console.log(`Building astesia-mcp (${profile}) for ${targetTriple}...`);

  const cargo = spawn('cargo', cargoArguments, {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      // Building the sidecar runs this package's Tauri build script. Disable
      // externalBin validation for this inner build; the executable is staged
      // immediately after Cargo finishes.
      TAURI_CONFIG: JSON.stringify({ bundle: { externalBin: [] } }),
    },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  const completion = new Promise((resolveExit, rejectExit) => {
    cargo.once('error', rejectExit);
    cargo.once('close', resolveExit);
  });
  const forwardInterrupt = () => cargo.kill('SIGINT');
  const forwardTermination = () => cargo.kill('SIGTERM');
  process.once('SIGINT', forwardInterrupt);
  process.once('SIGTERM', forwardTermination);
  const lines = createInterface({ input: cargo.stdout });
  let executable;

  try {
    for await (const line of lines) {
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        process.stderr.write(`${line}\n`);
        continue;
      }

      if (message.reason === 'compiler-message' && message.message?.rendered) {
        process.stderr.write(message.message.rendered);
      }

      if (
        message.reason === 'compiler-artifact'
        && message.target?.name === 'astesia-mcp'
        && message.target?.kind?.includes('bin')
        && message.executable
      ) {
        executable = message.executable;
      }
    }
  } finally {
    process.off('SIGINT', forwardInterrupt);
    process.off('SIGTERM', forwardTermination);
  }

  const exitCode = await completion;

  if (exitCode !== 0) {
    throw new Error(`cargo build failed with exit code ${exitCode}`);
  }
  if (!executable) {
    throw new Error('cargo completed without reporting the astesia-mcp executable');
  }

  return executable;
}

async function main() {
  const arguments_ = process.argv.slice(2);
  const unexpectedArguments = arguments_.filter((argument) => argument !== '--debug');
  if (unexpectedArguments.length > 0) {
    throw new Error(`Unknown argument(s): ${unexpectedArguments.join(', ')}`);
  }

  const configuredTargetTriple = [
    process.env.TAURI_ENV_TARGET_TRIPLE,
    process.env.TAURI_TARGET_TRIPLE,
  ]
    .map((value) => value?.trim())
    .find(Boolean);
  const targetTriple = configuredTargetTriple || readHostTriple();

  if (!/^[A-Za-z0-9_.-]+$/.test(targetTriple)) {
    throw new Error(`Invalid Rust target triple: ${JSON.stringify(targetTriple)}`);
  }
  if (targetTriple === 'universal-apple-darwin') {
    throw new Error(
      'universal-apple-darwin requires separate arm64/x86_64 builds and lipo; '
      + 'build one concrete target triple at a time',
    );
  }

  const debug = arguments_.includes('--debug') || isEnabled(process.env.TAURI_ENV_DEBUG);
  const profile = debug ? 'dev' : 'release';
  const extension = targetTriple.includes('windows') ? '.exe' : '';
  const destination = resolve(
    binariesDirectory,
    `astesia-mcp-${targetTriple}${extension}`,
  );

  await mkdir(binariesDirectory, { recursive: true });
  const executable = await buildSidecar(targetTriple, profile);
  await copyFile(executable, destination);
  if (!extension) {
    await chmod(destination, 0o755);
  }

  console.log(`Prepared Tauri sidecar: ${destination}`);
}

main().catch((error) => {
  console.error(`Failed to prepare astesia-mcp sidecar: ${error.message}`);
  process.exitCode = 1;
});
