import fs from "node:fs";
const file = "package.json";
const json = JSON.parse(fs.readFileSync(file));
delete json.packageManager;
fs.writeFileSync(file, JSON.stringify(json, null, 4));
