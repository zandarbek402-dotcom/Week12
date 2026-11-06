// Builds target/idl/liquidity_vault.json from the output of
//   cargo test __anchor_private_print_idl --features idl-build -- --show-output --quiet
// (the same thing `anchor idl build` does). Used here because the Anchor CLI
// was not available on the build machine. With Anchor CLI just run `anchor build`.
const fs = require("fs");

const raw = fs.readFileSync(process.argv[2] ?? "target/idl/raw.txt", "utf8");
const sections = [...raw.matchAll(/--- IDL begin (\w+) ---\r?\n([\s\S]*?)\r?\n--- IDL end \1 ---/g)];

let idl;
const events = [];
const types = new Map();
let errors = [];
let address = "";

for (const [, kind, body] of sections) {
  const json = JSON.parse(body);
  if (kind === "program") idl = json;
  else if (kind === "address") address = JSON.parse(json);
  else if (kind === "errors") errors = json;
  else if (kind === "event") {
    events.push(json.event);
    for (const t of json.types ?? []) types.set(t.name, t);
  } else if (kind === "type") {
    for (const t of json.types ?? []) types.set(t.name, t);
  }
}

for (const t of idl.types ?? []) types.set(t.name, t);
idl.address = address;
idl.events = [...(idl.events ?? []), ...events].sort((a, b) => a.name.localeCompare(b.name));
idl.errors = errors.length ? errors : idl.errors;
idl.types = [...types.values()].sort((a, b) => a.name.localeCompare(b.name));

fs.mkdirSync("target/idl", { recursive: true });
fs.writeFileSync("target/idl/liquidity_vault.json", JSON.stringify(idl, null, 2));
fs.mkdirSync("idl", { recursive: true });
fs.copyFileSync("target/idl/liquidity_vault.json", "idl/liquidity_vault.json");
console.log(
  `IDL: ${idl.instructions.length} instructions, ${idl.accounts?.length ?? 0} accounts, ` +
    `${idl.events.length} events, ${idl.errors.length} errors -> target/idl/liquidity_vault.json`
);
