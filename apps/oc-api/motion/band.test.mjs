import assert from "node:assert/strict";
import test from "node:test";
import { bandCopy, chromeLaunchFailed, readFigure } from "./render.mjs";

test("a comma number keeps its label", () => {
  const copy = bandCopy("10,000 prize pool", "");
  assert.equal(copy.figure.n, 10000);
  assert.equal(copy.label, "prize pool");
  assert.equal(copy.line, "");
});

test("a bare number uses the prompt as the label", () => {
  const copy = bandCopy("10000", "prize pool");
  assert.equal(copy.figure.n, 10000);
  assert.equal(copy.label, "prize pool");
});

test("a name or a quote has no figure", () => {
  const name = bandCopy("Asha", "");
  assert.equal(name.figure, null);
  assert.equal(name.line, "Asha");
  const quote = bandCopy("the work is the point", "");
  assert.equal(quote.figure, null);
  assert.equal(quote.line, "the work is the point");
});

test("a profile rmdir is a chrome startup failure", () => {
  const gone = new Error("ENOTEMPTY: directory not empty, rmdir '/tmp/oc-chrome-2ZoKow/Default'");
  gone.code = "ENOTEMPTY";
  assert.equal(chromeLaunchFailed(gone), true);
  assert.equal(chromeLaunchFailed(new Error("chrome exited before debugging: 1")), true);
  assert.equal(chromeLaunchFailed(new Error("ffmpeg failed")), false);
});

test("percent and dollars stay on the figure", () => {
  const pct = readFigure("47%");
  assert.equal(pct.n, 47);
  assert.equal(pct.suffix, "%");
  const money = readFigure("$10,000");
  assert.equal(money.n, 10000);
  assert.equal(money.prefix, "$");
  assert.equal(bandCopy("1.50 km", "").figure.decimals, 2);
});
