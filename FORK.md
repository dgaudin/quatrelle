# Ce fork : dgaudin/mailcraft

Ce dépôt est un fork de [ZIDAN44/mailcraft](https://github.com/ZIDAN44/mailcraft). Il s'en écarte
volontairement sur l'apparence, pour ressembler davantage à Outlook et s'intégrer au bureau, et il
ajoute des fonctions Exchange. Ce fichier dit **quoi**, **pourquoi**, et **comment revenir en
arrière** si besoin.

Point de départ : le commit `8cbd116` (« Docs: v0.15 »). Les modifications sont sur la branche
`claude/practical-gauss-g3t65c`.

## 1. Les icônes : Fluent UI System Icons de Microsoft

**L'écart.** Le projet d'origine interdit toute illustration Microsoft : ses icônes sont des tracés
dessinés dans le code. Ce fork utilise à la place les **Fluent UI System Icons** de Microsoft, que
ce soit pour le ruban, les modules ou les menus.

**Pourquoi.** On voulait des icônes libres de droit, bien connues des projets web, et proches de
celles d'Outlook. Fluent est la famille d'icônes du Outlook actuel, et Microsoft la publie sous
licence **MIT** ([microsoft/fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons)).
On peut donc l'utiliser, la modifier et la redistribuer, à condition de garder la licence.

**Comment.**
- Les fichiers sont dans `assets/icons/fluent/` : les contours « regular » de 20 pixels, copiés
  **sans modification** depuis le paquet npm `@fluentui/svg-icons` 1.1.292. La licence MIT est
  dans `assets/icons/fluent/LICENSE.txt`.
- Chaque fichier a sa ligne dans `ATTRIBUTION.md`. C'est exigé par `cargo xtask assets`, qui
  bloque le CI sinon.
- `crates/ui-egui/src/icons.rs` associe chaque icône à son fichier. Le dessin se fait avec
  `resvg`, une bibliothèque en Rust pur qui fonctionne aussi dans le navigateur. L'icône est
  rendue une fois par taille, puis colorée au moment de l'affichage.
- Les anciens tracés en code restent en secours, si un SVG ne peut pas être rendu.
- `CLAUDE.md`, `AGENTS.md` et `README.md` mentionnent cette exception comme une décision du
  propriétaire.

**Les limites, qui restent valables.**
- Aucune icône « classique » d'Office, ni aucune image, aucun son, aucun modèle ou texte d'Outlook.
- L'icône de l'application et sa couleur restent celles de MailCraft.
- Une nouvelle icône doit être un autre fichier Fluent, ajouté de la même façon : fichier
  inchangé, plus une ligne dans `ATTRIBUTION.md`.

**Revenir en arrière.** Annuler le commit `69bef61`, ainsi que les ajouts d'icônes des commits qui
le suivent. Les tracés en code reprennent alors automatiquement.

## 2. La fenêtre suit le bureau

**L'écart.** Le projet d'origine dessinait sa propre fenêtre : une barre grise sans titre sous
GNOME Wayland, sa couleur sarcelle partout, sa police, et un mode clair forcé sous Linux.

**Pourquoi.** Une application de bureau doit avoir les mêmes boutons de fenêtre (réduire,
agrandir, fermer), les mêmes couleurs et les mêmes polices que les autres applications.

**Comment** (commit `3f5a21d`, code dans `apps/mailcraft/src/appearance.rs`).
- **Barre de titre.** Là où le gestionnaire de fenêtres la dessine (KDE, wlroots, X11, Windows,
  macOS), c'est lui qui le fait. Sous GNOME Wayland, la fonction `wayland-csd-adwaita` de `winit`
  en dessine une identique à celle de GNOME : titre, boutons placés selon le réglage
  `button-layout`, clair ou sombre.
- **Clair ou sombre.** Sous Linux, le réglage est lu via le portail XDG (GNOME et KDE). Avant,
  « Comme le système » donnait toujours le mode clair.
- **Couleur d'accent.**
  - Elle vient du système : DWM sous Windows, l'accent de macOS, `accent-color` du portail sous
    Linux. Elle remplace le sarcelle de MailCraft.
  - Elle est éclaircie ou foncée jusqu'à rester lisible sur les volets.
  - **Option, activée par défaut** : Options › « Utiliser la couleur d'accentuation du système ».
    Décochez-la pour retrouver le sarcelle.
- **Police.**
  - Sous Linux, c'est celle de l'interface du bureau : `font-name` de GNOME, `kdeglobals` de KDE,
    sinon le `sans-serif` de fontconfig.
  - Elle n'est prise que si elle a un gras.
  - Aucune police n'est embarquée.
- **Mise à jour.** Le style est relu toutes les quelques secondes : un changement dans les
  réglages du bureau apparaît sans redémarrer.

## 3. L'apparence d'Outlook

Le modèle est le Outlook classique, **observé de l'extérieur** à partir de captures d'écran de sa
fenêtre. Ces captures ne sont pas dans le dépôt, et rien n'a été lu dans l'installation d'Outlook.
Seules la disposition et les noms des fonctions sont repris.

| Élément | Ce qui change | Commit | Option pour revenir |
|---|---|---|---|
| Barre du haut | Neutre (couleur des volets) au lieu d'être remplie de couleur d'accent ; recherche centrée | `2d678e2` | — |
| Ruban | Grands boutons avec leur nom dessous, rangés en groupes nommés | `3a89df5` | — |
| Ruban repliable | Se replie sur ses onglets avec ⌃ ou Ctrl+F1 ; état gardé d'un lancement à l'autre | `a55c4e9` | le ⌃ / Ctrl+F1 |
| Modules | Rail vertical à gauche (Courrier, Calendrier, Contacts…) | `7c06664` | — |
| Liste des messages | Tableau à colonnes triables quand la liste est large | `0e537a4` | Affichage › Liste des messages : Automatique / Cartes / Tableau |
| Volet des dossiers | Noms seuls, sans icône devant, lignes plus serrées | `a55c4e9` | — |
| Barre To-Do | À droite du courrier : le mois en cours et les rendez-vous de la semaine | `0237901` | Affichage › Barre des tâches |
| Calendrier | Vue Semaine de travail, en-têtes de jours, deux mois dans le volet, groupes de calendriers | `2c30e94` | — |

**Pourquoi « Automatique » pour la liste.** Outlook montre des cartes quand le volet de lecture est
à droite (la liste est étroite), et un tableau quand il est en bas ou masqué. Le mode Automatique
fait la même chose ; les deux autres choix forcent l'un ou l'autre.

## 4. Fonctions Exchange ajoutées

Toutes s'appuient sur les spécifications publiques de Microsoft (EWS, Graph, [MS-OXOMSG],
[MS-OXOCFG]), pas sur le code d'Outlook.

- **Boutons de vote** (`1fb5aaf`)
  - Les messages avec des boutons de vote les affichent dans le volet de lecture.
  - La réponse est envoyée comme Outlook l'attend (`PidLidVerbResponse`), pour que l'expéditeur la
    voie comptée.
  - Fonctionne sur Exchange (EWS) et sur Microsoft 365 (Graph).
- **Boîtes partagées et délégations** (`27e3dbe`)
  - Fichier › Ouvrir une autre boîte aux lettres.
  - La boîte s'affiche sous un dossier portant son adresse, avec son calendrier.
  - Un message supprimé va dans ses propres Éléments supprimés.
- **Groupes de calendriers**
  - Les groupes du volet Calendrier d'Outlook (« Équipe : … », « Calendriers partagés »,
    salles) sont repris avec les calendriers des collègues qu'ils contiennent.
  - Sur Microsoft 365 (`54bba09`).
  - Sur Exchange **on-premise** (`0817f83`), à partir des raccourcis de navigation qu'Outlook
    enregistre dans la boîte, décrits dans [MS-OXOCFG] §2.2.9.

## 5. Construire un paquet (.rpm, .deb, AppImage…)

- **Sur GitHub.** Les Actions sont activées sur ce fork. Le workflow
  **Release (unsigned test build)** (`.github/workflows/release-wip.yml`) se lance à la main :
  Actions › Run workflow, en choisissant la branche. Il publie une pré-version
  `v<version>-test.<n>` avec tous les paquets, non signés.
- **En local.** Avec [nfpm](https://nfpm.goreleaser.com) installé :
  `packaging/linux/package.sh --formats rpm`. Le paquet sort dans `dist/release/`.

## 6. Ce qui ne change pas

- La règle « ne jamais planter » : pas de `unwrap()` ni de `panic!` hors des tests, et chaque
  correction de plantage vient avec son test.
- Les mots de passe restent dans le trousseau du système.
- Les vérifications de certificats restent toujours actives.
- Le contenu distant n'est jamais chargé sans action de l'utilisateur.
- Tout le texte de l'interface passe par `tl!` / `tf!`, traduit dans les 14 langues.
- `cargo xtask ci` doit rester vert avant chaque commit.
- Les identifiants d'application (`ai.storyteller.mailcraft`), les noms des exécutables et le
  `UpgradeCode` de l'installeur Windows sont inchangés.

## Suivre le projet d'origine

Pour récupérer les nouveautés de ZIDAN44/mailcraft :

```sh
git remote add upstream https://github.com/ZIDAN44/mailcraft
git fetch upstream
git merge upstream/main
```

Les conflits les plus probables concernent les fichiers touchés par ce fork :
`crates/ui-egui/src/icons.rs`, `widgets.rs`, `lib.rs`, `list.rs`, `calendar.rs`, `ATTRIBUTION.md`
et `CLAUDE.md`. Gardez la version de ce fork pour les icônes et le style, et celle d'origine pour
le reste.
