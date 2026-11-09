# Nomad

Nomad este un MVP desktop local-first pentru construirea aplicațiilor cu ajutorul unui agent AI. Folosește Tauri 2, un backend Rust și o interfață React/TypeScript — fără Electron.

## Ce funcționează în MVP

- creare și deschidere proiecte locale;
- explorer de fișiere protejat împotriva path traversal și symlink escape;
- citire, căutare și propuneri de modificări fără scriere imediată;
- chat OpenAI prin Responses API, cu streaming de evenimente către UI;
- propuneri AI în format structurat, diff, acceptare sau respingere;
- editor simplu care transformă salvarea într-o propunere aprobabilă;
- dev-server pornit ca proces copil, loguri, timeout pentru comenzile one-shot și oprire controlată;
- preview localhost în aplicație;
- snapshot-uri Git și rollback confirmat;
- proiecte, mesaje, propuneri și setări ne-secrete în SQLite;
- cheia OpenAI în credential vault-ul sistemului prin `keyring`.

## Pornire

Prerechizite: Node 20+, Rust stable, WebView2 și dependențele Tauri pentru Windows.

```powershell
npm install
npm run tauri dev
```

La prima pornire:

1. Deschide **Setări**, introdu cheia OpenAI și opțional modelul.
2. Creează un proiect într-un director părinte sau deschide un folder existent.
3. Selectează un fișier și discută cu agentul. O modificare propusă nu atinge discul până la **Acceptă**.
4. Configurează comanda dev-server (implicit `npm run dev -- --host 127.0.0.1`) și pornește preview-ul.

Agentul poate propune o singură modificare per răspuns folosind blocul intern:

```text
<nomad-change>{"path":"src/App.tsx","summary":"...","content":"..."}</nomad-change>
```

Blocul este extras de backend și persistat ca propunere; interfața afișează diff-ul înainte de aprobare.

## Validare

```powershell
npm run test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

## Limite cunoscute / etapele următoare

- bucla agentului expune uneltele Rust, dar MVP-ul folosește deliberate „single proposed change”, nu execuție autonomă multi-tool;
- diff-ul UI este line-based, nu un editor Monaco și nu are merge pe hunk;
- rollback-ul restaurează starea unui commit Git; proiectele cu operații Git externe concurente necesită atenție;
- preview-ul presupune o adresă localhost introdusă de utilizator; detectarea automată a portului urmează;
- terminal PTY interactiv, index semantic, worktrees și suport pentru provideri suplimentari sunt în etapa următoare;
- nu există încă packaging/signing CI pentru Windows, macOS și Linux.

## Model de securitate

Toate uneltele de fișiere primesc căi relative la proiect. Backend-ul respinge componente absolute/`..`, verifică rădăcina canonicalizată și refuză traversarea prin symlink-uri. Comenzile nu trec prin shell: programul și argumentele sunt transmise separat. Scrierile, acceptarea unei propuneri, rularea comenzilor, pornirea proceselor și rollback-ul cer un token de confirmare scurt, unic și consumabil.

