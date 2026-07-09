//! `powder new <dir>` — the create-powder-app project scaffolder.

use std::path::Path;

use crate::codegen;
use crate::schema::{Schema, SAMPLE_SCHEMA};

const SEED_JSON: &str = r#"{
  "users": [
    { "id": 1, "name": "alice", "score": 9.5, "active": true },
    { "id": 2, "name": "bob", "score": null, "active": false }
  ],
  "posts": [
    { "id": 1, "user_id": 1, "title": "hello powder" },
    { "id": 2, "user_id": 1, "title": "second post" }
  ]
}
"#;

const PACKAGE_JSON: &str = r#"{
  "name": "powder-app",
  "private": true,
  "type": "module",
  "scripts": {
    "generate": "powder generate --ts src/models.ts",
    "migrate": "powder migrate --db app.db",
    "seed": "powder seed --db app.db --file seed.json",
    "validate": "powder validate --db app.db",
    "build": "powder validate --db app.db && tsc -p tsconfig.json",
    "start": "node src/main.js"
  },
  "dependencies": {
    "@powder/node": "*"
  },
  "devDependencies": {
    "typescript": "^5.4.0"
  }
}
"#;

const TSCONFIG_JSON: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "strict": true,
    "skipLibCheck": true
  },
  "include": ["src/**/*.ts"]
}
"#;

const MAIN_TS: &str = r#"import { Client } from "@powder/node";
import { powder } from "./models.js";

const client = await Client.connect("app.db");
const db = powder(client);

// Simple lookups: by primary key, or chain filters step by step.
console.log("user 1:", (await db.users.find(1))?.name);

const top = await db.users
  .where({ active: true })
  .orderBy("score", "desc")
  .limit(5)
  .all();
console.log("top users:", top.map((u) => u.name));

// Relations are loaded in one extra query each — no N+1.
const posts = await db.posts.findMany({
  include: { user: true },
  orderBy: { id: "asc" },
});
for (const post of posts) {
  console.log(`#${post.id} ${post.title} — by ${post.user?.name ?? "?"}`);
}

// Custom named queries live in powder.schema.json; their SQL is compiled
// when you run `powder generate`.
console.log("topUsers:", await db.$queries.topUsers({ active: true, minScore: 5 }));

// Transactions roll back automatically on throw (nested ones use savepoints).
await db.$transaction(async (tx) => {
  await tx.users.create({ id: 3, name: "carol", score: 7.5, active: true });
  await tx.posts.create({ id: 3, user_id: 3, title: "carol's post" });
});

console.log("users:", await db.users.count());
"#;

const GITIGNORE: &str = "node_modules/\n*.db\nsrc/*.js\n";

const README: &str = r#"# Powder app

```bash
npm install
npm run migrate    # create tables from powder.schema.json
npm run seed       # load seed.json
npm run generate   # AOT-generate src/models.ts
npm run build      # schema gate (powder validate) + tsc
npm run start
```

Edit `powder.schema.json`, then re-run `npm run migrate && npm run generate`.
Destructive changes (dropped columns / type changes) need
`powder migrate --db app.db --rebuild`.
"#;

/// Create a new Powder project under `dir`. Fails if `dir` already exists.
pub fn scaffold(dir: &str) -> Result<Vec<String>, String> {
    let root = Path::new(dir);
    if root.exists() {
        return Err(format!("`{dir}` already exists"));
    }
    let schema = Schema::parse(SAMPLE_SCHEMA).expect("sample schema is valid");

    let files: Vec<(&str, String)> = vec![
        ("powder.schema.json", SAMPLE_SCHEMA.to_string()),
        (
            // Editor autocompletion/validation for powder.schema.json.
            "powder.schema.schema.json",
            crate::jsonschema::SCHEMA_OF_SCHEMA.to_string(),
        ),
        ("seed.json", SEED_JSON.to_string()),
        ("package.json", PACKAGE_JSON.to_string()),
        ("tsconfig.json", TSCONFIG_JSON.to_string()),
        (".gitignore", GITIGNORE.to_string()),
        ("README.md", README.to_string()),
        ("src/main.ts", MAIN_TS.to_string()),
        ("src/models.ts", codegen::typescript(&schema, "@powder/node")),
    ];

    let mut written = Vec::with_capacity(files.len());
    for (rel, contents) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, contents).map_err(|e| e.to_string())?;
        written.push(format!("{dir}/{rel}"));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_writes_a_complete_project() {
        let dir = std::env::temp_dir().join(format!("powder-new-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.to_string_lossy().to_string();

        let written = scaffold(&dir_s).unwrap();
        assert_eq!(written.len(), 9);
        assert!(dir.join("powder.schema.schema.json").exists());
        assert!(dir.join("src/models.ts").exists());
        assert!(scaffold(&dir_s).is_err()); // refuses to overwrite

        let models = std::fs::read_to_string(dir.join("src/models.ts")).unwrap();
        assert!(models.contains("export function powder"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
