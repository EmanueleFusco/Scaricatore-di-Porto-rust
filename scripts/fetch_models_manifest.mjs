#!/usr/bin/env node
// Validate and query the Whisper models catalogue (whisper.models.json).
//
// Usage:
//   node scripts/fetch_models_manifest.mjs --check    # Offline schema check
//   node scripts/fetch_models_manifest.mjs --verify   # Verify sha256 & sizes against Hugging Face

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import process from "node:process";

const MANIFEST_PATH = resolve("whisper.models.json");

export function validateModelsCatalogue(catalogue) {
  const problems = [];
  if (catalogue.schemaVersion !== 1) {
    problems.push(`schemaVersion is ${catalogue.schemaVersion}, expected 1`);
  }

  if (!catalogue.vad) {
    problems.push("catalogue has no 'vad' entry");
  } else {
    if (!catalogue.vad.id) problems.push("vad: missing id");
    if (!catalogue.vad.file) problems.push("vad: missing file");
    if (!catalogue.vad.url || !catalogue.vad.url.startsWith("https://")) {
      problems.push("vad: url must be an https URL");
    }
    if (typeof catalogue.vad.sizeBytes !== "number" || catalogue.vad.sizeBytes <= 0) {
      problems.push("vad: sizeBytes must be a positive number");
    }
    if (!/^[0-9a-f]{64}$/.test(catalogue.vad.sha256 ?? "")) {
      problems.push("vad: sha256 must be 64 lowercase hex characters");
    }
  }

  if (!Array.isArray(catalogue.models) || catalogue.models.length === 0) {
    problems.push("catalogue has no models");
    return problems;
  }

  const seenIds = new Set();
  const seenFiles = new Set();

  for (const model of catalogue.models) {
    if (!model.id) {
      problems.push("a model has an empty id");
      continue;
    }
    if (seenIds.has(model.id)) problems.push(`duplicate model id: ${model.id}`);
    seenIds.add(model.id);

    if (!model.label) problems.push(`${model.id}: missing label`);
    if (!model.file) problems.push(`${model.id}: missing file`);
    if (seenFiles.has(model.file)) problems.push(`duplicate model file: ${model.file}`);
    seenFiles.add(model.file);

    if (!model.url || !model.url.startsWith("https://")) {
      problems.push(`${model.id}: url must be an https URL`);
    }
    if (typeof model.sizeBytes !== "number" || model.sizeBytes <= 0) {
      problems.push(`${model.id}: sizeBytes must be a positive number`);
    }
    if (!/^[0-9a-f]{64}$/.test(model.sha256 ?? "")) {
      problems.push(`${model.id}: sha256 must be 64 lowercase hex characters`);
    }
    if (!model.ramEstimate) problems.push(`${model.id}: missing ramEstimate`);
    if (!model.tier) problems.push(`${model.id}: missing tier`);
  }

  return problems;
}

async function verifyAgainstHuggingFace(catalogue) {
  process.stdout.write("Checking Hugging Face tree API for whisper.cpp models...\n");
  const res = await fetch("https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/main");
  if (!res.ok) {
    throw new Error(`Failed to query Hugging Face API: HTTP ${res.status}`);
  }
  const tree = await res.json();

  let mismatches = 0;
  for (const model of catalogue.models) {
    const item = tree.find((t) => t.path === model.file);
    if (!item) {
      process.stderr.write(`  MISSING on Hugging Face: ${model.file}\n`);
      mismatches += 1;
      continue;
    }
    const actualSize = item.size;
    const actualSha = item.lfs?.oid;
    if (actualSize !== model.sizeBytes) {
      process.stderr.write(`  SIZE MISMATCH: ${model.file}: catalogue has ${model.sizeBytes}, HF has ${actualSize}\n`);
      mismatches += 1;
    }
    if (actualSha !== model.sha256) {
      process.stderr.write(`  SHA256 MISMATCH: ${model.file}: catalogue has ${model.sha256}, HF has ${actualSha}\n`);
      mismatches += 1;
    }
    if (actualSize === model.sizeBytes && actualSha === model.sha256) {
      process.stdout.write(`  ok: ${model.file} (${model.sizeBytes} bytes, sha256: ${model.sha256.slice(0, 12)}…)\n`);
    }
  }

  if (catalogue.vad) {
    process.stdout.write("Checking Hugging Face tree API for whisper-vad models...\n");
    const vadRes = await fetch("https://huggingface.co/api/models/ggml-org/whisper-vad/tree/main");
    if (vadRes.ok) {
      const vadTree = await vadRes.json();
      const vadItem = vadTree.find((t) => t.path === catalogue.vad.file);
      if (vadItem) {
        if (vadItem.size !== catalogue.vad.sizeBytes || vadItem.lfs?.oid !== catalogue.vad.sha256) {
          process.stderr.write(`  VAD MISMATCH for ${catalogue.vad.file}\n`);
          mismatches += 1;
        } else {
          process.stdout.write(`  ok: ${catalogue.vad.file} (VAD)\n`);
        }
      }
    }
  }

  if (mismatches > 0) {
    throw new Error(`${mismatches} mismatch(es) found between catalogue and Hugging Face`);
  }
}

async function main() {
  const mode = process.argv[2] ?? "--check";
  const raw = readFileSync(MANIFEST_PATH, "utf8");
  const catalogue = JSON.parse(raw);

  const problems = validateModelsCatalogue(catalogue);
  if (problems.length > 0) {
    for (const p of problems) process.stderr.write(`  - ${p}\n`);
    process.exit(1);
  }

  process.stdout.write(`whisper.models.json OK: ${catalogue.models.length} model(s), VAD model present, schema ${catalogue.schemaVersion}\n`);

  if (mode === "--verify") {
    await verifyAgainstHuggingFace(catalogue);
    process.stdout.write("All catalogue sizes and SHA-256 hashes match Hugging Face upstream.\n");
  }
}

if (process.argv[1] && process.argv[1].endsWith("fetch_models_manifest.mjs")) {
  main().catch((err) => {
    process.stderr.write(`fetch_models_manifest: ${err.message}\n`);
    process.exit(1);
  });
}
