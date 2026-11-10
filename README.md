# Nomad

Nomad este o aplicație desktop local-first pentru construirea și editarea proiectelor cu ajutorul unui agent AI. Folosește Tauri 2, un backend Rust și o interfață React/TypeScript — fără Electron.

## Funcții disponibile

- creare și deschidere proiecte locale;
- explorer de fișiere protejat împotriva path traversal și symlink escape;
- citire, listare, căutare, patch și scriere atomică în workspace;
- chat cu streaming prin API-uri compatibile OpenAI Chat Completions, Claude Code CLI sau Codex CLI;
- propuneri AI în format structurat, diff, acceptare sau respingere;
- editor simplu care transformă salvarea într-o propunere aprobabilă și detectează schimbările concurente;
- conversații persistente, sesiuni multiple și anulare stream;
- dev-server pornit ca proces copil, loguri limitate, timeout pentru comenzile one-shot și oprirea arborelui de procese pe Windows;
- preview localhost validat și iframe izolat;
- diff local față de ultimul snapshot;
- snapshot-uri Git și rollback confirmat;
- export de proiect într-un folder ales, cu cheile, fișierele `.env`, directoarele Git și artefactele de build omise;
- proiecte, mesaje, propuneri și setări ne-secrete în SQLite;
- centru de setări pe categorii, profil local și trei accente vizuale persistente;
- verificare la pornire și instalare de update-uri Tauri semnate, cu verificare manuală din Setări → Actualizări;
- cheile API în credential vault-ul sistemului prin `keyring`; CLI-urile Claude Code/Codex folosesc login-ul lor local existent fără ca Nomad să citească sau să copieze tokenurile;
- proiect starter cu preview localhost inclus, pentru pornire fără configurare npm.

## Instalare și platforme

Repository-ul include un workflow GitHub Actions care construiește pachete pentru Windows x64, Linux x64, macOS Apple Silicon și macOS Intel/universal. Pe un tag `v*`, pachetele se atașează automat la GitHub Release; `workflow_dispatch` produce artifacte de build.

| Sistem | Fișiere de instalare |
| --- | --- |
| Windows x64 | `.msi` și `.exe` (NSIS) |
| Linux x64 | `.deb` și `.AppImage` |
| macOS Apple Silicon / Intel | `.dmg` nativ și universal |

Pachetele macOS/Linux trebuie construite pe runners nativi. Semnarea și notarizarea macOS nu sunt configurate încă; macOS poate cere autorizarea manuală la prima deschidere. Pentru distribuție publică, adaugă certificatele de semnare în GitHub Secrets.

### Website Nomad

Landing page-ul de produs este în `website/`, separat de UI-ul desktop. Pentru publicare pe Vercel, importă repository-ul, setează **Root Directory** la `website`, păstrează Framework Preset **Other** și lasă Build Command necompletată. Site-ul este static și include installerul Windows preview în `website/downloads/`; instalatoarele macOS/Linux se vor adăuga după buildurile native și validarea lor.

Actualizările în aplicație încep după instalarea primei versiuni care include updaterul și configurarea secretului de semnare descrisă în [RELEASE_GUIDE.md](RELEASE_GUIDE.md). Release-urile noi generează un feed `latest.json` cu semnăturile platformelor.

Pentru pașii compleți de publicare a release-urilor, instalare pe fiecare OS și publicarea website-ului produsului, vezi [RELEASE_GUIDE.md](RELEASE_GUIDE.md).

### Dezvoltare locală

Windows: Node 20+, Rust stable, Visual Studio C++ Build Tools și WebView2.

Linux: Node 20+, Rust stable, WebKitGTK 4.1, appindicator, librsvg, OpenSSL și libsecret development packages.

macOS: Node 20+, Rust stable și Xcode Command Line Tools.

```sh
npm install
npm run tauri dev
```

Pachete locale se construiesc cu `npm run build:windows`, `npm run build:linux`, `npm run build:macos:apple` sau `npm run build:macos:intel` pe sistemul/runner-ul potrivit. Tauri pune instalatoarele în `src-tauri/target/<target>/release/bundle/`.

La prima pornire:

1. Deschide **Setări → Furnizori AI**. Alege API compatibil OpenAI, Claude Code CLI sau Codex CLI. Pentru serverele locale fără autentificare poți lăsa cheia goală.
2. Creează un proiect într-un director părinte sau deschide un folder existent.
3. Selectează un fișier și discută cu agentul. O modificare propusă nu atinge discul până la **Acceptă**.
4. Pentru proiecte existente configurează comanda dev-server și adresa localhost. Proiectul starter inclus folosește `node dev-server.mjs` și `http://127.0.0.1:5173`.

Pentru utilizare gratuită, poți alege un model marcat gratuit în OpenRouter (cheia OpenRouter rămâne necesară) sau poți instala Ollama și folosi `http://localhost:11434/v1` fără cheie; descarcă separat un model local din Ollama. Claude Code/Codex pot folosi login-ul lor existent, dacă planul/contul tău permite folosirea CLI-ului.

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

- Agent Mode poate orchestra până la cinci apeluri read-only (`read_file`, `list_files`, `search_files`, `git_diff`) per mesaj, apoi poate propune o modificare completă de fișier pentru aprobare; editarea multi-fișier rămâne pentru o etapă ulterioară;
- un singur provider este activ pentru chat; mai multe profiluri API pot fi păstrate local, dar rutarea automată între modele nu este implementată;
- Claude Code și Codex sunt lansate prin CLI-ul instalat și autentificat de utilizator. Claude Code rulează restricționat, fără unelte sau persistența sesiunii; Codex rulează efemer, read-only, cu configul de unelte personalizat dezactivat. CLI-urile și versiunile lor diferite nu au fost validate pe toate sistemele;
- conectarea la servere MCP nu este implementată; comenzile și scrierile agentului nu se execută automat, iar fiecare schimbare propusă rămâne în fluxul de diff/aprobare;
- login, profil sincronizat, abonamente/credite, sync de date în Neon/Supabase, library, telemetry opt-in și deploy către hosting extern necesită configurare și integrare suplimentară;
- exportul proiectului este disponibil; importul/migrarea bazei locale între calculatoare și editorul Monaco cu merge pe hunk urmează;
- editorul este textarea cu diff line-based, fără Monaco și merge pe hunk;
- portul preview nu este detectat automat;
- terminal PTY interactiv, index semantic, worktrees și provideri suplimentari urmează;
- aplicația include workflow-uri pentru pachete native; build-ul și semnarea macOS/Linux se finalizează pe runners GitHub corespunzători.

Panoul de setări arată starea actuală a acestor integrări și nu salvează tokenuri pentru servicii care nu sunt active.

## Model de securitate

Toate uneltele de fișiere primesc căi relative la proiect. Backend-ul respinge componente absolute/`..`, verifică rădăcina canonicalizată și refuză traversarea prin symlink-uri. Fișierele de credențiale uzuale (`.env`, chei private și directoarele de configurare AI) sunt excluse din contextul modelelor, iar integrarea CLI primește o copie temporară a codului fără aceste fișiere. Comenzile din uneltele Nomad nu trec prin shell: programul și argumentele sunt transmise separat. Scrierile, acceptarea unei propuneri, rularea comenzilor, pornirea proceselor și rollback-ul cer un token de confirmare scurt, unic și consumabil.

## Backend cloud și monetizare

Arhitectura opțională pentru conturi, abonamente Stripe și registrul de credite este în [`cloud/README.md`](cloud/README.md). Aplicația desktop rămâne local-first până când sunt configurate un provider JWT, un proiect Neon dedicat și secretele Stripe pe server. Nu pune credențiale de producție în repository sau în build-ul desktop.

Secțiunile de setări permit acum păstrarea preferințelor locale; Integrări poate verifica endpoint-ul cloud, iar Zona periculoasă exportă preferințele și șterge cheia API. Autentificarea utilizatorilor și plata reală nu sunt active până la conectarea serviciilor externe și a fluxului de sign-in nativ.
