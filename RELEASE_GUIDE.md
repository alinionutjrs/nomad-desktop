# Ghid de build, distribuție și website Nomad

Acest ghid descrie cum să obții instalatoarele, să le publici și să folosești aplicația. Repository-ul curent este sursa proiectului; un website public și un release cu instalatoare vor necesita un repository GitHub și rularea workflow-ului.

## Fișierele proiectului

- Aplicația desktop: `src/` și `src-tauri/`.
- Configurație de build și identitate: `src-tauri/tauri.conf.json`.
- Workflow pentru instalatoare: `.github/workflows/release.yml`.
- Flux de actualizări semnate Tauri: feed-ul `latest.json` este publicat împreună cu fiecare release.
- Ghidul principal și funcțiile disponibile: `README.md`.
- Website-ul static de produs se află în `website/`; pentru Vercel folosește această cale ca Root Directory.

## Obținerea instalatoarelor

### Build automat pe GitHub

1. Creează un repository GitHub privat sau public și urcă folderul proiectului. Pentru colaborare sau distribuție publică, folosește un repository public.
2. În tabul **Actions**, activează GitHub Actions dacă GitHub îl solicită.
3. Deschide workflow-ul **Build desktop installers** și apasă **Run workflow**. Workflow-ul produce artifacte care pot fi descărcate din pagina run-ului; artifactele sunt temporare și nu reprezintă o pagină permanentă de download.
4. Pentru o pagină permanentă, creează și publică un tag de versiune, de exemplu `v0.1.1`. Workflow-ul atașează instalatoarele la GitHub Release.
5. După ce toate job-urile termină cu succes, deschide **Releases** și descarcă asset-urile pentru platformele dorite.

Comenzi Git uzuale pentru următoarea versiune (rulează din folderul repository-ului):

```sh
git add .
git commit -m "Prepare Nomad release"
git push
git tag v0.1.1
git push origin v0.1.1
```

Schimbă versiunea în `package.json`, `src-tauri/Cargo.toml` și `src-tauri/tauri.conf.json` înainte de fiecare release. Păstrează cheia de semnare existentă; utilizatorii vor accepta doar pachetele semnate de cheia publică inclusă deja în aplicație.

### Configurarea actualizărilor automate

Cheia privată generată pentru acest proiect este păstrată local în `src-tauri/keys/nomad-updater.key` și este ignorată de Git. Nu o adăuga în repository. În repository-ul GitHub, deschide **Settings → Secrets and variables → Actions → New repository secret** și creează:

- `TAURI_SIGNING_PRIVATE_KEY`: conținutul fișierului `src-tauri/keys/nomad-updater.key`.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: parola cheii, dacă ai generat o cheie cu parolă; pentru cheia actuală lasă secretul gol/neconfigurat.

Nu regenera și nu pierde cheia privată. Fără ea nu mai pot fi create release-uri acceptate de instalările existente. Cheia publică este deja inclusă în `src-tauri/tauri.conf.json` și backend.

Workflow-ul tag-uit generează artefacte updater semnate pentru Windows x64, Linux x64 și macOS Intel/Apple Silicon, construiește `latest.json`, apoi publică toate fișierele în GitHub Release. Endpoint-ul GitHub este inclus automat în buildurile făcute de Actions. În Nomad, utilizatorii pot verifica manual din **Setări → Actualizări**; aplicația verifică și la pornire și afișează butonul **Actualizează și repornește** când găsește o versiune nouă. Actualizarea se descarcă și se verifică prin semnătură înainte de instalare.

Prima versiune a Nomad care conține updaterul trebuie instalată manual peste versiunea curentă. Release-urile ulterioare se pot instala din aplicație.

### Ce pachet descarcă utilizatorii

| Platformă | Pachet recomandat | Instalare |
| --- | --- | --- |
| Windows 10/11 x64 | `.exe` (NSIS) pentru instalare simplă sau `.msi` pentru administrare IT | Rulează installer-ul și urmează pașii afișați. |
| Ubuntu/Debian x64 | `.deb` | `sudo apt install ./nomad_*.deb` |
| Alte distribuții Linux x64 | `.AppImage` | `chmod +x Nomad_*.AppImage && ./Nomad_*.AppImage`; poate fi nevoie de WebKitGTK și FUSE în funcție de distribuție. |
| Fedora/RHEL x64 | `.rpm` | `sudo dnf install ./nomad-*.rpm` |
| macOS Apple Silicon | DMG Apple Silicon sau universal | Deschide DMG și trage Nomad în Applications. |
| macOS Intel | DMG Intel sau universal | Deschide DMG și trage Nomad în Applications. |

Buildurile macOS din workflow nu sunt semnate/notarizate. Pentru distribuție către utilizatori obișnuiți, configurează Developer ID certificate și notarizare Apple în secretele GitHub; fără acestea, Gatekeeper poate bloca prima deschidere. Pachetele Linux și aplicația nu includ toate toolchain-urile de dezvoltare sau modele AI; utilizatorul trebuie să instaleze separat runtime-urile proiectelor și orice CLI AI dorește să folosească.

## Prima pornire

1. Descarcă installer-ul pentru sistemul și procesorul tău din **GitHub → Releases**.
2. Instalează și pornește Nomad.
3. Deschide setările AI și alege una dintre variante:
   - endpoint OpenAI-compatibil, de exemplu OpenRouter, furnizor custom sau server local Ollama;
   - Claude Code CLI instalat și autentificat separat;
   - Codex CLI instalat și autentificat separat.
4. Pentru API, introdu endpoint-ul, modelul și cheia furnizorului. Cheia se stochează în seiful de credențiale al sistemului.
5. Creează un proiect sau deschide un folder de proiect existent. Încearcă starter-ul inclus pentru un exemplu care pornește local.
6. Cere o schimbare în chat, inspectează diff-ul propus, apoi aprobă sau respinge.
7. Pentru proiecte existente, configurează comanda dev-server și adresa localhost în setările proiectului și pornește preview-ul. Proiectul starter Nomad pornește `node dev-server.mjs` la `http://127.0.0.1:5173`.

Un endpoint local Ollama poate fi folosit fără taxă per cerere după instalarea Ollama și descărcarea unui model local; costul practic este resursa hardware/energia. Planurile, limitele și costurile serviciilor terțe sunt stabilite de furnizorii respectivi. În această versiune există un singur endpoint API activ la un moment dat.

## Unde să publici aplicația și website-ul

- **Codul sursă:** GitHub repository. Păstrează-l privat cât timp produsul este în lucru; fă-l public dacă vrei contribuții open-source.
- **Instalatoarele:** GitHub Releases oferă o locație simplă de descărcare legată de cod și versiuni. Pentru volum mai mare poți adăuga ulterior un bucket/CDN și update feed.
- **Website produs:** landing page-ul din `website/` poate fi publicat pe Vercel ca site static, cu Root Directory `website`, preset `Other`, fără build command. Domeniul propriu și DNS-ul se configurează ulterior în Vercel. Site-ul conține installerul Windows preview; macOS/Linux se listează ca în pregătire până când există pachete native verificate.
- **Semnare software:** cumpără/configurează certificatul Apple Developer ID și semnarea Windows înainte de distribuție publică la scară. Nu distribui chei de semnare în repository; folosește secretele CI.

Acest repository nu are încă un website, domeniu, cont de hosting, analytics sau formular de contact. Acestea sunt pași separați de aplicația desktop și trebuie configurate înainte de lansarea publică.
