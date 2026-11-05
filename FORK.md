# Ce fork : dgaudin/quatrelle

Ce dépôt est un fork de [ZIDAN44/mailcraft](https://github.com/ZIDAN44/mailcraft), renommé
**Quatrelle** (son nom d'origine était MailCraft). Il s'en écarte
volontairement sur l'apparence, pour ressembler davantage à Outlook et s'intégrer au bureau, et il
ajoute des fonctions Exchange. Ce fichier dit **quoi**, **pourquoi**, et **comment revenir en
arrière** si besoin.

Point de départ : le commit `7d16bb1` (« Docs: v0.15 »). Les modifications sont sur `main`.

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
- L'icône de l'application et sa couleur restent celles de Quatrelle.
- Une nouvelle icône doit être un autre fichier Fluent, ajouté de la même façon : fichier
  inchangé, plus une ligne dans `ATTRIBUTION.md`.

**Revenir en arrière.** Annuler le commit `d00314f`, ainsi que les ajouts d'icônes des commits qui
le suivent. Les tracés en code reprennent alors automatiquement.

## 2. La fenêtre suit le bureau

**L'écart.** Le projet d'origine dessinait sa propre fenêtre : une barre grise sans titre sous
GNOME Wayland, sa couleur sarcelle partout, sa police, et un mode clair forcé sous Linux.

**Pourquoi.** Une application de bureau doit avoir les mêmes boutons de fenêtre (réduire,
agrandir, fermer), les mêmes couleurs et les mêmes polices que les autres applications.

**Comment** (commit `5ea491a`, code dans `apps/quatrelle/src/appearance.rs`).
- **Barre de titre.** Là où le gestionnaire de fenêtres la dessine (KDE, wlroots, X11, Windows,
  macOS), c'est lui qui le fait. Sous GNOME Wayland, la fonction `wayland-csd-adwaita` de `winit`
  en dessine une identique à celle de GNOME : titre, boutons placés selon le réglage
  `button-layout`, clair ou sombre.
- **Clair ou sombre.** Sous Linux, le réglage est lu via le portail XDG (GNOME et KDE). Avant,
  « Comme le système » donnait toujours le mode clair.
- **Couleur d'accent.**
  - Elle vient du système : DWM sous Windows, l'accent de macOS, `accent-color` du portail sous
    Linux. Elle remplace le sarcelle de Quatrelle.
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
| Barre du haut | Neutre (couleur des volets) au lieu d'être remplie de couleur d'accent ; recherche centrée | `ebff331` | — |
| Ruban | Grands boutons avec leur nom dessous, rangés en groupes nommés | `40309b8` | — |
| Ruban repliable | Se replie sur ses onglets avec ⌃ ou Ctrl+F1 ; état gardé d'un lancement à l'autre | `7b0e502` | le ⌃ / Ctrl+F1 |
| Modules | Rail vertical à gauche (Courrier, Calendrier, Contacts…) | `803e497` | — |
| Liste des messages | Tableau à colonnes triables quand la liste est large | `dee4c4b` | Affichage › Liste des messages : Automatique / Cartes / Tableau |
| Volet des dossiers | Noms seuls, sans icône devant, lignes plus serrées | `7b0e502` | — |
| Barre To-Do | À droite du courrier : le mois en cours et les rendez-vous de la semaine | `f360b7d` | Affichage › Barre des tâches |
| Calendrier | Vue Semaine de travail, en-têtes de jours, deux mois dans le volet, groupes de calendriers | `87c35ae` | — |
| Noms de dossiers longs | Sur une ligne, coupés par « … », le nom entier en infobulle (« Toutes les boîtes de réception » passait sur deux lignes) | `0686020c` | — |
| Texte caché d'un message | Une ligne discrète, « Texte caché : n mots · Afficher », l'explication en infobulle, au lieu d'un cadre coloré de trois lignes | `0686020c` | — |
| Liste de diffusion | L'adresse de désabonnement est un lien : page web dans le navigateur, ou nouveau message avec l'objet que la liste demande | `0686020c` | — |

**Pourquoi « Automatique » pour la liste.** Outlook montre des cartes quand le volet de lecture est
à droite (la liste est étroite), et un tableau quand il est en bas ou masqué. Le mode Automatique
fait la même chose ; les deux autres choix forcent l'un ou l'autre.

## 4. Fonctions Exchange ajoutées

Toutes s'appuient sur les spécifications publiques de Microsoft (EWS, Graph, [MS-OXOMSG],
[MS-OXOCFG]), pas sur le code d'Outlook.

- **Boutons de vote** (`f90e091`)
  - Les messages avec des boutons de vote les affichent dans le volet de lecture.
  - La réponse est envoyée comme Outlook l'attend (`PidLidVerbResponse`), pour que l'expéditeur la
    voie comptée.
  - Fonctionne sur Exchange (EWS) et sur Microsoft 365 (Graph).
- **Boîtes partagées et délégations** (`39fba06`)
  - Fichier › Ouvrir une autre boîte aux lettres.
  - La boîte s'affiche sous un dossier portant son adresse, avec son calendrier.
  - Un message supprimé va dans ses propres Éléments supprimés.
- **Groupes de calendriers**
  - Les groupes du volet Calendrier d'Outlook (« Équipe : … », « Calendriers partagés »,
    salles) sont repris avec les calendriers des collègues qu'ils contiennent.
  - Sur Microsoft 365 (`911c85d`).
  - Sur Exchange **on-premise** (`93468a2`), à partir des raccourcis de navigation qu'Outlook
    enregistre dans la boîte, décrits dans [MS-OXOCFG] §2.2.9.
- **Glisser-déposer dans le calendrier**
  - Un rendez-vous se déplace à la souris dans les vues Jour, Semaine de travail, Semaine et
    Mois (à la minute près, par quart d'heure) ; son bord inférieur change l'heure de fin ;
    Échap abandonne ; Ctrl+Z remet en place.
  - Une occurrence d'une série peut être déplacée seule (une instance avec `RECURRENCE-ID`) :
    CalDAV l'écrit dans le fichier de la série ; Exchange (EWS) demande l'élément de
    l'occurrence (`OccurrenceItemId`) et le modifie, puis relit avec la série ses occurrences
    modifiées (`ModifiedOccurrences`) et supprimées (`DeletedOccurrences`). Sur Microsoft 365
    (Graph), seule la série entière se déplace : Graph ne relit pas les exceptions.
  - Pour une réunion organisée par l'utilisateur, la question d'Outlook : « Envoyer la mise à
    jour / Enregistrer sans envoyer ». Sur Exchange, c'est le serveur qui envoie la mise à jour
    (`SendToAllAndSaveCopy`, sur la clé de modification de l'élément), aux invités qu'il connaît
    désormais (`RequiredAttendees`) ; plus d'invitation iMIP en double. Sur Microsoft 365, le
    serveur prévient toujours. Ailleurs (CalDAV, calendrier local), un `REQUEST` par courrier,
    `SEQUENCE` incrémenté.
- **Rappels**
  - La notification de bureau d'un rappel reste jusqu'à réponse, sonne, et a les boutons
    « Répéter dans 5 minutes » et « Ignorer » (Linux, via `notify-rust`) ; un clic dessus ramène
    la fenêtre.
  - Option « Afficher les rappels au-dessus des autres fenêtres » (activée par défaut) : la
    fenêtre Rappels s'ouvre dans une fenêtre à part, toujours au premier plan, même Quatrelle
    réduit (`RequestPaintWhileHidden` tant qu'un rappel est dû).

### Exchange Online : inscription d'application propre, boîtes partagées, détection

Trois ajouts (2026-10-22) pour Microsoft 365 (Graph), détaillés dans `docs/oauth.md` :

- **Inscription d'application de l'organisation.** L'identifiant d'application (client) ne venait
  que de `QUATRELLE_MICROSOFT_CLIENT_ID` (à la compilation ou dans l'environnement), et le
  locataire était figé à `common`. Les deux se saisissent désormais **par compte**, dans Ajouter
  un compte › Paramètres du serveur › « Connexion Microsoft : l'inscription d'application de votre
  organisation », et dans les paramètres du compte ensuite. Ils sont gardés dans `accounts.json`
  (`oauth_client_id`, `oauth_tenant`, sans secret) et remplacent l'identifiant intégré et
  `common` pour ce compte. Une organisation peut ainsi inscrire Quatrelle elle-même, en
  locataire unique, et faire approuver l'application par son administrateur ; une copie de
  Quatrelle compilée sans aucune inscription ajoute quand même un tel compte. En ligne de
  commande : `oauthClientId` et `oauthTenant` sur `account.add` et `account.update`.
- **Boîtes partagées.** Les permissions Graph demandées comprennent `Mail.ReadWrite.Shared`,
  `Mail.Send.Shared` et `Calendars.ReadWrite.Shared`, pour que les boîtes et calendriers
  partagés s'ouvrent. Une permission ajoutée oblige Microsoft à redemander le consentement à la
  prochaine connexion : le navigateur s'ouvre une fois de plus, rien n'est perdu.
- **Détection de Microsoft 365.** Quand l'enregistrement MX du domaine est celui d'Exchange
  Online (`<domaine>.mail.protection.outlook.com`), ou que son enregistrement
  `_autodiscover._tcp` pointe vers Microsoft, le dialogue d'ajout propose **Exchange
  (Microsoft 365)** par Graph plutôt qu'IMAP, souvent désactivé dans ces organisations. Les
  réglages IMAP trouvés par ailleurs restent sous Paramètres du serveur.

## 5. Construire un paquet (.rpm, .deb, AppImage…)

- **Sur GitHub.** Les Actions sont activées sur ce fork. Le workflow
  **Release (unsigned test build)** (`.github/workflows/release-wip.yml`) se lance à la main :
  Actions › Run workflow, en choisissant la branche. Il publie une pré-version
  `v<version>-test.<n>` non signée, pour Linux (rpm, deb, AppImage, Flatpak, tar.gz) et le Web.
  Windows, macOS et FreeBSD ne sont construits que pour les versions stables (décision du
  28/10/2026 : leurs constructions étaient les plus longues et faisaient attendre chaque
  version de test).
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
- Les identifiants d'application (`ai.storyteller.quatrelle`), les noms des exécutables et le
  `UpgradeCode` de l'installeur Windows sont inchangés.

## Suivre le projet d'origine

Pour récupérer les nouveautés de ZIDAN44/mailcraft, la méthode qui a servi pour la v0.22 (voir
plus bas) est la plus sûre : une fusion directe bute sur le renommage MailCraft → Quatrelle à
chaque ligne que l'origine touche.

```sh
git fetch https://github.com/ZIDAN44/mailcraft main && git branch -f upstream-main FETCH_HEAD
```

Les conflits les plus probables concernent les fichiers touchés par ce fork :
`crates/ui-egui/src/icons.rs`, `widgets.rs`, `lib.rs`, `list.rs`, `calendar.rs`, `ATTRIBUTION.md`
et `CLAUDE.md`. Gardez la version de ce fork pour les icônes et le style, et celle d'origine pour
le reste.

### État de la récupération : v0.17 intégrée (2026-10-19)

`upstream/main` a été fusionnée dans la branche de ce fork : 42 commits (v0.16, v0.17, egui 0.37,
Rust 1.95 minimum). Les deux versions n'ont eu aucun commit divergent — `main` n'avait rien de
propre — donc la fusion n'a rien réécrit. 19 fichiers ont demandé une résolution :

- **Traductions** (5 `.tsv`) : les deux côtés ajoutaient des clés différentes ; elles sont toutes
  gardées, sans doublon.
- **Cache des lignes** (`list.rs`) : le cache de l'origine (perf des grosses boîtes) est gardé, et
  sa clé inclut la hauteur du mode table de ce fork.
- **Filets de sécurité** : `Cargo.lock` et `ROADMAP.md` viennent de l'origine (le journal du fork y
  est ajouté) ; `logfile.rs` est la version de l'origine (rotation, tests).
- **Ajouts des deux côtés** : `widgets.rs` (vos widgets de ruban + `switch`/`segmented`), `ews.rs`,
  `ews_pim.rs`, `fake_ews.rs`, `fake_graph.rs` gardent les champs et fonctions des deux.
- **Envois Exchange** : la connexion rapide de l'origine (une deuxième connexion par compte)
  **remplace** le thread par message de ce fork. Même service — envoyer pendant une synchronisation —
  mais dans l'ordre, et les réponses automatiques en profitent. C'était la seule vraie divergence de
  conception ; trois tests en dépendent, ils passent.
- **Fusion non retenue** : la page File › Info garde la mise en page de l'origine (cartes et
  réglages), tandis qu'Open & Export garde les tuiles de ce fork. À harmoniser si souhaité.

Preuves de la fusion : `cargo xtask ci` vert (fmt, `clippy -D warnings`, 366 tests, layers, assets,
wasm), un build `--workspace` complet, et le test d'envoi Exchange relancé trois fois sans échec.

### État de la récupération : v0.18 et v0.19 intégrées (2026-10-19)

Second passage, sans recréer de remote : `git fetch https://github.com/ZIDAN44/mailcraft main` puis
fusion de `FETCH_HEAD` (10 commits, v0.18 + v0.19 — boîtes partagées Exchange, fiche contact,
sign-in SMTP propre, fichier `.quatrelle` conservé à la suppression d'un compte). Aucun commit de
ce fork n'a été réécrit ; 23 fichiers ont demandé une résolution.

- **Traductions** (14 `.tsv`) : union des deux côtés, sans doublon.
- **Modèle** (`model/lib.rs`) : le `Folder` prend les champs de l'origine `mailbox` et
  `mailbox_role` (une boîte partagée est une boîte à part entière). Le tri du volet unionne la
  règle de l'origine (Exchange, dossiers marqués) et celle de ce fork (Graph, dossiers nommés
  d'après l'adresse, détectés par le `@`).
- **Exchange (EWS)** : l'implémentation de l'origine **remplace** celle de ce fork — elle lit les
  boîtes partagées qu'Autodiscover associe au compte en plus de celles ajoutées à la main, avec les
  vrais rôles de dossiers. Le `address_of` de ce fork reste (il sert aux calendriers).
- **Graph** : ce fork garde son implémentation (boîtes partagées sous `/users/<adresse>`, que
  l'origine n'a pas), plus le `name_special` de l'origine.
- **Boîte du même rôle** (`session.rs`) : `folder()` et `folder_beside()` reconnaissent les dossiers
  d'une autre boîte des deux façons (`mailbox` de l'origine, préfixe d'adresse de ce fork), et
  `move_selection` groupe par boîte — un message supprimé dans une boîte partagée Exchange va bien
  dans *ses* Éléments supprimés. Un test le vérifie désormais sur les deux formes de nommage.
- **Dialogues et boîtes partagées** : unifiés sur l'entrée de ce fork (commit `51f110a`) —
  « Ouvrir une autre boîte aux lettres… », plus riche (Graph et calendrier), garde ses commandes
  `account.openMailbox`/`account.closeMailbox` ; le dialogue et les commandes de l'origine
  (`AddSharedMailbox`, `account.addSharedMailbox`/`removeSharedMailbox`) sont retirés. Le menu d'une
  boîte partagée ferme désormais via `account.closeMailbox`, et `shared_mailboxes` se compare sans
  tenir compte de la casse (une entrée laissée par l'ancienne voie restait sinon impossible à fermer).
- **Doublure de test** (`fake_ews.rs`) : celle de l'origine (boîtes partagées, ancrage
  `X-AnchorMailbox`) plus les ajouts de ce fork (boutons de vote, dossiers masqués, `slow_sync`,
  répertoire, raccourcis de navigation), `ResolveNames` réuni.

Preuves de la fusion : `cargo xtask ci` vert (fmt, `clippy -D warnings`, 375 tests, layers, assets,
wasm) et un build `--workspace` complet. Après l'unification des dialogues, `cargo xtask ci` reste
vert (374 tests — un test de l'origine retiré avec ses commandes). La version du manifeste suit
l'origine (`0.19.0`).

### État de la récupération : v0.21, v0.22 et le découpage en petits fichiers (2026-10-22)

Les 51 commits de l'origine (`600f97f..8ba8253`) sont fusionnés en trois fusions, dans une
branche à part puis dans `main` : jusqu'à `215ce10` (les fonctions de la v0.22), puis le découpage
de l'origine en petits fichiers (`215ce10..55c5853`), puis le reste (`..8ba8253`). L'historique de
l'origine est un parent de chaque fusion : la prochaine repartira de `8ba8253`.

**Méthode.**
- **Renommer l'origine d'abord.** Chaque commit de l'origine utilisé comme point de fusion est
  recopié avec MailCraft → Quatrelle appliqué aux chemins et au texte (comme `0f88a0e` l'avait
  fait ici), et la fusion se fait contre ces copies (`git merge-tree --merge-base`). Vérifié : le
  renommage automatique de `c744fa8` redonne `0f88a0e` à l'identique, hors ajouts voulus
  (`legacy.rs`, icônes). Il ne reste alors que les vrais conflits.
- **Le découpage de l'origine, élément par élément.** Quand l'origine éclate un fichier que ce fork
  a modifié, chaque fonction, méthode ou test du fork est fusionné à trois voies dans le fichier
  où l'origine l'a mis ; ce que le fork a ajouté va à côté de son voisin. Les commandes du fork vont
  dans les registres par domaine (`cmd/*.rs`), ses dialogues dans les groupes (`dialogs/*.rs`).
- **Rejouable.** Chaque étape est un script : le rejouer redonne exactement le même arbre. Les
  outils (Python, et `itemize`, un petit programme Rust qui liste les éléments d'un fichier) sont
  hors du dépôt, dans `~/mailcraft/merge-tools/`.

**Décisions.**
- **Gardé de ce fork** : l'apparence (ruban, rail, icônes Fluent, fenêtre qui suit le bureau), le
  calendrier et les contacts, et tout ce que ce fork avait déjà porté à la main (instance unique,
  en-tête étroit, modifications gardées pendant une synchro, IPv6/IPv4, semaine de travail, boîtes
  partagées, recherche sur le serveur EWS et IMAP), ainsi que la fiabilité Exchange (`b023484`).
- **Pris de l'origine** : Quick Steps (groupe « Actions rapides » du ruban, icône Fluent `flash`),
  expéditeurs bloqués et approuvés, Clean Up, dossiers de recherche, `larger:`/`smaller:`, rappels
  avec répétition et la fenêtre Rappels (à la place de la sonnerie simple de ce fork), plusieurs
  messages en cours de rédaction, réglages enregistrés au fil de l'eau, animations, recherche sur le
  serveur pour Graph et JMAP, portées de recherche et recherche avancée, Graph (ImmutableId, 429),
  la mise à jour automatique des versions de test — pointée sur `dgaudin/quatrelle`.
- **Les deux** : le `session/mailboxes.rs` de ce fork (autres boîtes ouvertes) devient
  `session/shared_mailboxes.rs`, l'origine ayant un module de ce nom. L'index de recherche de ce
  fork (construit par tranches) couvre les portées de l'origine ; l'onglet Rechercher garde ses
  boutons radio empilés, pour les quatre portées.
- **La règle des petits fichiers** de l'origine (`cargo xtask sizes` : 1000 lignes par fichier,
  16 fichiers par dossier) s'applique aussi à ce fork : les fichiers qu'il avait fait grossir sont
  découpés (`2b8f930`), sans changement de comportement.

Preuves : `cargo xtask ci` vert à chaque étape (541 tests à la fin, fmt, `clippy -D warnings`,
tailles, couches, ressources, wasm), et la fenêtre regardée sur la boîte d'exemple.

### État de la récupération : reprises choisies de la v0.23 et de la v0.24 (2026-10-24)

L'origine a ajouté 60 commits après `8ba8253` (jusqu'à `49ad25c`). Ils ne sont pas fusionnés :
ils sont **repris un par un**, comme des *cherry-picks* des copies renommées
(`refs/merge/r-<commit>`, faites par `renamed.sh`), le diff entre la copie d'un commit et celle de
son parent appliqué à trois voies (`git apply -3`). La prochaine fusion complète repartira de
`8ba8253` : les changements repris à l'identique s'y retrouveront des deux côtés.

- **Repris** : les fuseaux horaires (`b9f0546`, `c4df00c`) et les invitations (`65d64b9`,
  `b34d158`, `4889315`) ; la revue de durcissement (`44dae91`, `014c2e6`, `9dffd7e`, `406fe3f`,
  `fd17e44`, `748b96d`, `e79ef90`, `1c0f7aa`, `303eda0`, `1a36d25`) ; le courrier plus sûr
  (`c433c66`, `d354be7`, `9954938`, `249b9f7`, `8912219`, `29a2459`) ; `mailto:` et les fichiers
  `.eml`/`.ics`/`.vcf` (`ebd873d`, `7db4066`, `f592eb9`, `0fbc402`) ; S/MIME de `a23eb9c` à
  `3866bdb`, puis `44d9506` sans ses lignes Windows.
- **S/MIME sans l'exception `unsafe`** (décision du propriétaire, 2026-10-24) : `1a30852` (le
  magasin de certificats Windows et le déchiffrement EC du trousseau macOS, seuls fichiers
  `unsafe` du crate `keys`) n'est pas repris, ni `897fb51` qui s'en sert. `crates/keys` garde les
  lints de l'espace de travail (`unsafe_code = "forbid"`). Signer, chiffrer, lire, les `.p12`, les
  cartes à puce (PKCS #11) et le trousseau macOS pour les clés RSA marchent.
- **Laissés pour la fusion complète** : les plafonds et IDLE d'IMAP (`7fbd05e`, `9083d2f`,
  `565f0bb`), qui visent la connexion IMAP que ce fork a remaniée ; le fuzz et `cargo-deny`
  (`9418429`, `5bf776e`) ; le découpage du worker et le renouvellement OAuth (`6b7fc80`,
  `d4eddd7`) ; les tests contre de vrais serveurs (`610c87c`) ; les numéros de version et la
  documentation de l'origine.
- **Déjà faits ici** : `7848644` (le fichier de données ouvert une seule fois) et `551f748` (la
  page Options dans son propre fichier).
- **Adaptations** : les rappels restent ceux de ce fork (dans `ical/alarms.rs`) ; les domaines
  connus des alertes viennent du carnet d'adresses de ce fork ; Chiffrer et Signer sont dans
  l'onglet Options du ruban du message, comme chez Outlook ; le `Keychain` reste
  `desktop/keychain.rs` ; quelques fichiers de ce fork passent en sous-dossier pour la règle des
  16 fichiers (`pim/scheduling.rs`, `addr/providers.rs`, `thread/voting.rs`,
  `desktop/appearance.rs`, `desktop/keychain.rs`), joignables par les mêmes chemins. Les
  certificats de test de l'origine gardent le nom « MailCraft Test CA » (signés, ils ne changent
  pas d'un octet).
- **Ajouté ici, côté Exchange** : le fuseau des séries lu et renvoyé (`StartTimeZone`, demandé
  nommément : `AllProperties` ne le donne pas, vérifié sur le serveur du propriétaire), et les
  champs d'un rendez-vous créé dans l'ordre du schéma EWS.


### État de la récupération : reprises choisies de la v0.25 et de la v0.26 (2026-10-26)

L'origine a ajouté 32 commits après `49ad25c` (jusqu'à `e46504c`). Même méthode : repris un par un
depuis les copies renommées, avec `~/mailcraft/merge-tools/pick.sh <commit>` (le diff de `refs/merge/r-<parent>` à
`refs/merge/r-<commit>`, appliqué par `git apply -3`).

- **Repris** : la synchronisation qui ne perd plus une modification faite après un renommage
  quand la liste des dossiers échoue (`e46504c`) ; S/MIME : pour quel certificat un message
  chiffré a été écrit (`eadd066`), les anciens certificats gardés pour lire le courrier qui leur
  était chiffré, listés dans Fichier › Informations › Sécurité, aussi dans le navigateur (`6a2eb5f`,
  `b8d4b9e`, `681542b`, `554901b`, `eac2ab2`, issue #6), et les signatures qui restent valides
  quand un relais retire `X-Mailer` (`47b89f1`).
- **Adaptations** : sans les boutons du magasin de certificats Windows et du trousseau macOS
  (`681542b`), ni les tests Windows ou contre de vrais serveurs, que ce fork n'a pas repris ; le
  bouton de carte à puce reste où il était.
- **Laissés** (décision du propriétaire, 2026-10-22 : l'apparence et le calendrier de ce fork
  restent) : le ruban « classique » et le mode sombre de l'origine (`b273cf8`, `b048d72`,
  `1cacf30`, issue #8) ; le serveur CalDAV donné à la main (`95c8779`, `695972d`, `fadbcc7`,
  `f6d66c5`, issue #7) ; le fuzz (`bab68ee`) ; la CI et la documentation de l'origine.
- **La mise à jour intégrée** (accord du propriétaire, 2026-10-26) : le téléchargement qui reprend
  où il s'était arrêté, la barre « prête » avec Redémarrer maintenant, la question à la fermeture,
  les AppImages là où l'utilisateur peut écrire (`2e4b4ed`, `e79f121`, `2b2ccb9`, `5e0620b`,
  `9e33e21`, `0533388`, `18a8a97`). Les fichiers du moteur de mise à jour sont ceux de l'origine,
  avec les versions de ce fork (`dgaudin/quatrelle`) ; la page À propos dans
  `backstage/about.rs`. `f6119f0` (les services du bureau dans `desktop.rs`) était déjà fait ici.
- **Les reprises laissées en v0.23 et v0.24, faites** (demande du propriétaire, 2026-10-26, avant
  d'y brancher Gmail et Yahoo) : les plafonds d'IMAP par commande (`7fbd05e` ; le téléchargement
  d'avance de ce fork, `sync/ahead.rs`, prend les mêmes lots bornés en taille), IDLE et la boîte de
  réception synchronisée à l'instant (`9083d2f`), le code de réponse suivi d'un texte non ASCII
  (`565f0bb`), le jeton OAuth renouvelé une fois avant de redemander la connexion (`d4eddd7`),
  cargo-deny (`5bf776e`), le fuzz (`9418429`, `bab68ee`, ses cibles IMAP et JMAP adaptées à ce
  fork) et les tests contre de vrais serveurs (`610c87c`). Le découpage du worker de l'origine
  (`6b7fc80`) n'est pas repris : celui de ce fork l'est déjà, autrement.

### État de la récupération : reprises choisies de la v0.27 (2026-10-27)

L'origine a ajouté 50 commits après `e46504c` (jusqu'à `d826960`). Même méthode, un par un depuis
les copies renommées (`~/mailcraft/merge-tools/pick.sh`, puis `run_picks.sh` qui applique,
compile et committe chaque reprise propre, et s'arrête au premier conflit).

- **Repris** : l'accessibilité (chaque contrôle dessiné a un nom pour les lecteurs d'écran ; les
  boutons propres à ce fork aussi, `633fd44`), la fermeture demandée même fenêtre réduite
  (`519dee1`, dans le `logic` de ce fork, qui lit l'état de la mise à jour d'abord), ⌘Q sur Mac
  (`3eab186`), le cache qui réécrit un fichier tenu un instant par un autre programme (`aec0eb1`,
  `f436878`), les dossiers relatifs ignorés (`7a7466b`), les noms de périphériques Windows
  (`7880728`), la boîte de dialogue qui rouvre le dernier dossier (`bab188a`), les images et pages
  PDF jamais plus grandes que la carte graphique (`4a04574`), le défilement du ruban par onglet
  (`e0c2d64`), le curseur au-delà du texte (`ebd2856`), l'hébreu et l'arabe dans les champs
  (`25e6033`, `ea13b4e`), sélectionnés et copiés dans un message (`8b77776`), les paragraphes de
  droite à gauche contre le bord droit (`754ab47`, en texte brut aussi), la place d'une image dans
  le texte (`156cd84`), la barre d'état aux nombreux comptes en attente (`fcdfbff`), le test « au
  hasard » de la fenêtre (`0d830b1`), des traductions (`c63c587`, `7800e8d`), l'exemple rempli de
  nouveau (`e3ccec5`), l'identité JMAP créée quand il n'y en a pas (`37a5ff0`), l'empaquetage et les
  polices du navigateur (`ff097b1`, `f1e7465`, `8e7d9b8`, `ccfacd8`, `8d9679b`, `8955bb3`,
  `3927f7e`, `42eba80`, `4799f8a`, `7342667`, `f95d2f3`, `82542f4`).
- **winit et egui partagés** : winit vient de `storytold/winit` (révision `5506b1a9`, la copie de
  `vendor/` est retirée, `4177c89`), egui de `storytold/egui` (`8f6d82d`), comme les autres
  applications Craft. Le winit partagé apporte le dépôt de fichiers sous Wayland (W15).
- **Adaptés** : le User-Agent propre à un compte Exchange et le nom d'utilisateur à saut de ligne
  refusé sont repris de `9ade2bb` sans sa recherche dans l'annuaire, ce fork ayant la sienne
  (Vérifier les noms, Liste d'adresses globale, l'autre adresse du champ De) ; le User-Agent va
  aussi sur les requêtes propres à ce fork (boîtes partagées, notifications).
- **Rattrapé** : `ca7c5e6` (de la v0.26, passé inaperçu alors), dont `ccfacd8` avait besoin : il
  donne à `GLIBC_FLOOR` sa valeur par défaut (2.35). Sans lui, le .rpm des tests 78 à 80 demandait
  `libc.so.6(GLIBC_)(64bit)`, que rien ne fournit, et dnf le refusait. `package.sh` refuse
  désormais toute variable de `nfpm.yaml` restée vide.
- **Laissés** : la mise en miroir de la fenêtre en arabe et en hébreu (`d2e280e` : elle réécrit la
  liste, les dossiers, la lecture et la rédaction, que ce fork dessine à sa façon) ; la couleur
  d'accent, la barre du haut sobre et les bordures (`cf15073`, l'apparence de ce fork reste) ;
  Kerberos (`c2f1788` : il réécrit la boucle de connexion EWS, que ce fork a renforcée autrement,
  et c'est le chemin NTLM du propriétaire) ; l'annuaire de Microsoft 365 (`81fa8d4`, posé sur le
  moteur d'annuaire de l'origine) ; les certificats gardés dans le navigateur (`41f42af`) ;
  l'agenda et les contacts EWS lus séparément (de `9ade2bb` : l'agenda de ce fork est le sien) ;
  ce qui était déjà fait ici (`500e2fc`, le thème du bureau ; `217325c`, Ctrl+V d'une image) ; la
  CI, la version et la documentation de l'origine (`2d8b274`, `baa180a`, `daadfd6`, `db8851c`,
  `763cb9f`, `d826960`).

### État de la récupération : v0.28 (2026-10-28)

L'origine a publié la v0.28 : onze commits après `d826960`. Même méthode, un par un.

- **Repris tels quels** (le jour même) : l'empaquetage et la CI (`df0f10a`, l'icône de Quatrelle
  épinglée gardée après une mise à jour sous Windows ; `6df62e5`, une version sans craft-fonts
  refusée ; `3c2a9fa`, libxkbcommon-x11 dans le .deb et le .rpm ; `7dc8276`, les panneaux
  d'ouverture et d'enregistrement du Mac dans la langue de l'interface ; `7ce73f5`, le certificat
  de signature décodé dans un dossier privé ; `0619fe9`, l'écriture réservée au travail qui
  rédige la version ; `b21aa6a`, FreeBSD et Windows ARM64 aux seules publications et chaque
  semaine).
- **Adapté** : l'abonnement à un calendrier par son lien iCal ou `webcal://` (`29f1bec`, Ajouter
  un calendrier › Depuis Internet d'Outlook). Le modèle (`WebCalendar`, `calendar_link`), le
  protocole (`net/protocols/web_calendar.rs`, ETag et Last-Modified), les commandes
  (`calendar.subscribe`, `calendar.unsubscribe`, `calendar.refreshSubscribed`), la boîte de
  dialogue et la ligne du volet (lien et état au survol, Actualiser et Supprimer au clic droit)
  viennent de l'origine ; la lecture tourne dans `worker/pool.rs` de ce fork (l'origine a
  `threads.rs`). Décisions du propriétaire, dans le sillage d'Outlook : un lien vers un serveur
  du réseau local est permis (le lecteur `http::send`, pas celui des images réservé à
  l'Internet) ; un calendrier abonné **n'est pas** un calendrier du compte (`own_calendar`),
  donc ses rendez-vous sont mis de côté avec ceux des autres hors du calendrier (`parking.rs` ;
  une lecture survenue pendant qu'ils étaient de côté l'emporte au retour) et ses alarmes ne
  sonnent jamais ; le point d'entrée du ruban est un menu Ajouter un calendrier (Depuis le
  carnet d'adresses… — l'ancien bouton Ouvrir un calendrier — et Depuis Internet…), avec le
  lien S'abonner à un calendrier… du volet. La fusion de synchronisation de ce fork
  (`syncing.rs`) garde le calendrier abonné et ses rendez-vous à travers une synchronisation
  complète EWS/CalDAV, même quand un rendez-vous du serveur porte le même UID, et une occurrence
  publiée sans sa série n'est pas jetée comme orpheline. Le refus de supprimer dans un
  calendrier en lecture seule, que l'origine corrige au passage, était déjà là.
- **Adapté** : de `2f57946` (les personnes trouvées dans l'annuaire montrées par leur nom
  complet), seule la moitié moteur : un contact connu ici par sa seule adresse (ou nommé par
  elle, ou par sa partie utilisateur) prend le nom que donne l'annuaire (`fill_name`), dans
  Vérifier les noms — qui interroge désormais l'annuaire pour un tel contact au lieu de
  l'écrire aussitôt, comme Outlook consulte d'abord la liste d'adresses — et dans l'autre
  adresse du champ De ; un nom qu'on a donné soi-même est gardé.
- **Laissés** : la moitié EWS de `2f57946` (`ReturnFullContactData`, le nom d'affichage
  préféré au nom de la boîte) : ce fork a sa propre recherche dans l'annuaire, et le serveur du
  propriétaire nomme déjà les gens correctement ; la documentation et la version de l'origine
  (`70b6257`, `3345a18`).

### État de la récupération : v0.29 (fin) et v0.30, première partie : la fiabilité (2026-10-31)

L'origine a ajouté 33 commits à décider après `d826960` (jusqu'à `706ca2f2`, la v0.30). Même
méthode, un par un depuis les copies renommées, sur la branche `zidan-v0.30`. Cette première
partie prend ce qui touche à la fiabilité et aux données ; la suite (analyseur de `winmail.dat`,
codes Alt, Exchange, signatures…) vient dans les parties suivantes.

- **Repris** : la fermeture demandée tant qu'un message est encore en cours d'envoi, car il n'est
  pas encore dans la Boîte d'envoi et se perdait sans un mot (`b0db8e8f`, issue #19) ; le courrier
  envoyé garde sa copie là où le serveur IMAP n'a pas de dossier Éléments envoyés : il est créé
  (`Sent`, ou `INBOX.Sent` sur un serveur qui range tout dans la boîte de réception), abonné, les
  noms de dossier sont lus décodés (UTF-7 modifié), les Éléments envoyés des autres langues sont
  connus, et quand aucune copie ne peut être gardée, on le dit (`f5e12476`) ; le dossier que le
  serveur marque `\Sent` l'emporte, où qu'il soit (`fe8beb07`) ; la ligne de commande attend son
  propre message puis sa copie (`8330c072`) et dit quand l'application tient les données ouvertes
  (`9439007f`, issue #20) ; les réponses et transferts emportent toutes les images du message cité,
  par `cid:`, par `Content-Location` ou nom, ou écrites en `data:` ; une image du Web reste par son
  adresse sans être chargée, les images espionnes sont écartées, et un courrier sans en-tête
  `Date` n'est plus cité comme envoyé le 1er janvier 1970 (`411d1a89`, `d2b25e3f`, issues #10 et
  #18) ; le courrier en Shift_JIS, GBK, Big5 et EUC-KR se lit (`98d8ebd2` : `full_encoding` de
  mail-parser, donc `encoding_rs` et `multiversion_no_op` au `Cargo.lock` ; `cargo deny check`
  passe).
- **Résolu à la main, sans toucher à l'apparence, au calendrier ni aux contacts** : les
  catalogues et `tests/writing.rs` gardent les ajouts des deux côtés (`b0db8e8f`) ; le dialogue
  `LostOnClose` du parcours de dialogues de `tests.rs`, propre à ce fork, prend son nouveau champ
  `sending` ; le faux serveur IMAP de ce fork est découpé (`fake/commands.rs`), la règle « créer
  seulement sous `INBOX/` » y est portée (`f5e12476`) ; `quoted_pictures` vit dans `mime/src/lib.rs`
  près de `attachment_data` (l'origine l'a dans `attachments.rs`, que ramènera le découpage de
  `winmail.dat`) et son appel dans `reply_compose` se place à côté des lignes propres à ce fork
  (signature, boîte partagée, mots de citation) (`d2b25e3f`).
- **Pas repris tel quel** : `135f7be1` (chaque test d'images dans son dossier) : les deux tests
  concernés sont dans le `pictures.rs` de l'origine, que ce fork a remplacé par le sien (un seul
  dossier `picture-size`). La même faute était chez ce fork dans deux tests de réunion qui
  partageaient `meeting-send-waits` : le second a maintenant son dossier.
- **Règle des petits fichiers** : ces reprises ont fait passer `model/src/lib.rs` (1007 lignes) et
  `engine/src/tests/writing.rs` (1001) au-delà de la limite. Les dossiers et leurs rôles vont dans
  `model/src/folder.rs` (avec `folder/utf7.rs`, ré-exporté sous les mêmes noms) et les deux tests
  d'images citées dans `tests/writing/quoted.rs`, sans changement de comportement.

### Deuxième partie : le `winmail.dat` d'Outlook (2026-10-31)

Sur la branche `zidan-v0.30-tnef`, un seul commit de l'origine, le plus important de la tournée
pour le propriétaire : le courrier des expéditeurs Outlook arrive souvent avec un `winmail.dat`
(`application/ms-tnef`) que rien ne savait ouvrir.

- **Repris** : `99cee57a` (issue #18). Le lecteur TNEF (`mime/src/attachments/tnef.rs`,
  MS-OXTNEF) donne les fichiers du `winmail.dat` avec leur nom long, leur type et leur `Content-ID`, et un message
  Outlook joint en `.eml` ; le lecteur RTF (`tnef/rtf.rs`, MS-OXRTFCP et MS-OXRTFEX) décompresse
  le corps, en ressort l'HTML qu'Outlook y a enveloppé, ou fait d'un RTF propre un HTML simple
  (gras, italique, souligné, barré, liens, sauts de ligne). Les fichiers prennent la place du
  `winmail.dat` dans la liste des pièces jointes, par le même index que les autres : enregistrer,
  prévisualiser, transférer et citer les images passent par `attachment_data` sans autre
  changement ; la recherche trouve leurs noms et le texte. Rien n'est chargé du réseau : l'HTML
  sorti du RTF suit le chemin de tout courrier HTML (images du Web retenues). La cible de fuzz
  `tnef` vient avec son dictionnaire et ses graines.
- **Résolu à la main** : l'origine a sorti le code des pièces jointes de `lib.rs` vers
  `attachments.rs` ; le `lib.rs` de ce fork diffère (réponses dans `reply.rs`, liens, `fnv`…),
  donc ce déplacement est refait à la main : `AttachmentInfo`, les genres, la règle des parties
  incorporées et `has_real_attachments` passent tels quels, `attachment_data` prend l'index commun
  au `winmail.dat`, et `quoted_pictures` (avec `data_url`) rejoint `attachments.rs` comme chez
  l'origine, inchangé. `lib.rs` exporte aussi `decompress_rtf` et `rtf_body`, qu'appelle la cible
  de fuzz (l'origine ne les exporte que plus tard).
- **Corrigé après relecture** : la limite `MAX_PROPS` (4096) valait par liste de propriétés, mais
  un flux peut répéter ses listes : elle vaut maintenant pour le message et pour chaque fichier.
  Dans le lecteur RTF, `raw()` écrivait les balises d'un changement de lien avant de regarder la
  limite de 16 Mo (40 Mo écrits pour 160 Ko de RTF dans le test) ; l'instruction d'un champ
  n'était pas bornée quand ses caractères étaient écrits en `\uN` ; et chaque jeton recopiait les
  liens de son groupe (désormais partagés, `Rc<str>`). Un test de régression pour chacun.
- **Corrigé après une seconde relecture, mesurée** (défauts venus de l'origine) :
  - chaque fichier à `Content-ID` cherchait `cid:` dans tout le corps HTML : 500 fichiers et un
    corps de 16 Mo tirés de 49 Ko de `winmail.dat` coûtaient 13 s par lecture, à la
    synchronisation (sur le fil de l'interface), à l'ouverture et pour chaque image. Le corps est
    maintenant lu une fois (`shown_cids`) ;
  - chaque corps avait ses 16 Mo, chaque message joint, à chaque niveau, son corps et son `.eml` :
    30 messages joints de 3 Ko gardaient 503 Mo. Un seul budget (`Budget`) vaut désormais pour
    toute une lecture, pour tous les `winmail.dat` d'un message et tous les niveaux : 64 Mo, plus
    deux fois les octets du flux (ses vrais fichiers ne sont jamais coupés), et 500 fichiers au
    total. Ce qui ne tient pas est laissé, et le `winmail.dat` reste alors listé à côté de ses
    fichiers ;
  - un lien RTF était récrit à chaque changement de gras ou d'italique (2,8 Ko de RTF donnaient
    16 Mo d'HTML) : il reste ouvert maintenant. Seul le premier corps des `winmail.dat` d'un
    message est lu, les suivants ne donnent que leurs fichiers ;
  - le trombone compte ce que donne un `winmail.dat` (un `winmail.dat` qui ne tient que le corps
    n'en met plus), et `quoted_pictures` lit le message une seule fois pour toutes ses images.
- **Troisième relecture** :
  - la recherche des images du corps relisait encore tout le corps pour chaque `winmail.dat` du
    message (13 s pour 100 parties) ; chaque référence `cid:` était lue jusqu'à la longueur du plus
    long Content-ID (53 s pour un Content-ID de 700 Ko). Désormais, une passe par message, et des
    références d'au plus 1 Ko ;
  - le compte des fichiers pouvait passer sous zéro quand un message joint prenait le dernier
    (panique en debug, et plus de limite en release) : le fichier est compté avant d'être lu ;
  - un lien RTF montré puis retiré à répétition récrivait son adresse chaque fois : au plus 1 Mo
    d'adresses par corps, puis le texte seul ;
  - le trombone ne décode plus de corps, et ne compte plus les images qu'Outlook marque comme
    celles du corps.
- **Règle des petits fichiers** : `mime/src/` serait passé à 17 fichiers Rust ; le lecteur TNEF
  (avec `tnef/rtf.rs` et ses tests) va sous `attachments/`, puisqu'un `winmail.dat` est une pièce
  jointe (l'origine l'a à `mime/src/tnef.rs`) ; les noms exportés ne changent pas.
- **Hors du `winmail.dat`** : deux paires de tests du moteur partageaient leur dossier temporaire
  (`contact-fields`, `server-search`) ; le second de chaque paire a le sien.

### Troisième partie : les codes Alt de Windows (2026-10-31)

Sur la branche `zidan-v0.30-alt`, un commit de l'origine, demandé par le propriétaire pour les
utilisateurs français : un clavier français n'a pas de touche pour É, È ou Ç, que Windows fait
taper par Alt+0201, Alt+0200, Alt+0199 sur le pavé numérique, et rien n'était tapé (issue #23 de
l'origine : Windows fait le caractère au relâchement d'Alt, et winit l'écarte).

- **Repris** : `7f70b72a`. Les chiffres tapés avec Alt seul tenu sont retirés de l'entrée de la
  fenêtre et le caractère est tapé au relâchement d'Alt, dans n'importe quel champ : avec un 0 en
  tête, dans la page ANSI du système (Windows-1252 en français : `0201` → É, `0128` → €) ; sans,
  dans sa page OEM (850 en français : `130` → é) ; au-delà de 255, en Unicode (`8364` → €). Une
  autre touche abandonne le code, et quitter la fenêtre en cours de code (Alt+Tab) ne tape rien
  dans celle qu'on quitte ; un code qui ne nomme rien (0, caractère de contrôle, trop long) ne
  tape rien. La lecture des pages est dans
  `mime/src/files/alt_code.rs`, le relevé des touches dans `ui-egui/src/app/alt_code.rs`.
- **Adapté** : l'origine appelle ce relevé depuis la structure `ClipboardKeys` de son collage de
  fichiers par Ctrl+V (`9b896ef3`, pas repris). Ce fork n'avait pas encore de crochet sur l'entrée
  des fenêtres : `main.rs` installe sous Windows seulement une structure `WindowInput` par
  `viewport_hooks` d'eframe (le correctif E1 de l'egui épinglé), qui passe sur la fenêtre
  principale (et la fenêtre de compactage, qui y est dessinée) comme sur chaque fenêtre ouverte
  par `show_viewport_immediate` : messages, rédaction détachée, réunions, rappels.
  `App::raw_input_hook` aurait manqué ces dernières. Chaque fenêtre garde son propre code en
  cours. Sous Linux, macOS et dans le navigateur, rien ne change. Le test de l'origine va dans
  `tests/writing/typing.rs` (le dossier `tests/` a déjà 16 fichiers).
- **Corrigé** (défaut venu de l'origine) : là où le système envoie aussi le texte d'un chiffre tapé
  avec Alt, ce texte n'était retiré que pendant un code ; après une touche qui l'avait interrompu
  (Alt+0, Alt+Tab, puis 2, 0, 1), « 201 » arrivait dans le texte. Le texte qui suit un chiffre
  retiré est retiré avec lui. Les index dans les tables OEM sont des soustractions vérifiées.
- **Corrigé après relecture** (sur le vrai trajet des événements de winit et d'egui-winit ; les
  trois premiers défauts venaient de l'origine) :
  - Alt+Tab au milieu d'un code tapait le code entamé dans la fenêtre qu'on quitte (Alt+1, 3 :
    ♪ ; Alt+6, 5 : A) : winit envoie les modificateurs relâchés *avant* la perte du focus, à la
    suite. Un relâchement d'Alt suivi de la perte du focus ne tape plus rien ;
  - la répétition d'un chiffre tenu arrive d'egui-winit comme d'autres pressions, marquées non
    répétées (egui calcule la répétition après le crochet) : un chiffre tenu plus d'une
    demi-seconde comptait plusieurs fois. Chaque fenêtre retient les chiffres enfoncés, et une
    pression ne compte qu'après le relâchement de sa touche ;
  - les pages venaient de la langue d'affichage (`sys_locale`), alors que Windows lit les codes
    dans les pages du système, que fixent les paramètres régionaux : un Windows affiché en anglais
    avec des paramètres français (courant en entreprise) prenait l'OEM 437 au lieu du 850, et
    Alt+183 donnait ╖ au lieu de À (de même È, Ê, Â, Ô). Sous Windows, les pages sont lues dans le
    registre (`ACP` et `OEMCP` de `HKLM\SYSTEM\CurrentControlSet\Control\Nls\CodePage`, par
    `windows-registry`, que l'app utilisait déjà : API sûre, MIT OR Apache-2.0, rien de neuf au
    `Cargo.lock` hormis la ligne de ui-egui ; `cargo deny check` passe) ; une page illisible pour
    les codes (OEM 852, japonais 932…) garde la supposition d'après la langue ;
  - AltGr sur un clavier AZERTY arrive sans aucun modificateur (winit efface Ctrl et Alt tant que
    l'Alt de droite est tenu) : le comportement était juste, un test le dit maintenant.
- **Vérifié par les tests** (sous Linux : seul l'appel est propre à Windows) : É, È, Ç et € en
  1252, é et É en 850, € en Unicode, la répétition d'un chiffre tenu ; rien pour les codes qui ne
  nomment rien ; Alt seul, Alt+F (les touches des menus), Ctrl+Alt et AltGr (sans modificateur
  sur AZERTY) passent tels quels ; Alt+Tab en cours de code ne tape rien ; un chiffre tenu compte
  une fois ; chaque fenêtre garde son code ; pages du registre préférées à la langue
  d'affichage. Les tests suivent l'ordre et la forme réels des événements. Le contrôle de compilation pour Windows ne passe ici
  que pour `quatrelle-mime` et `quatrelle-ui-egui` (le script de `ring` veut les outils MSVC) ;
  l'application est vérifiée par la porte Windows de GitHub. Reste à l'essayer avec de vraies
  touches dans une version de test Windows.
- **Limites connues, laissées telles quelles** : comme chez l'origine, les chiffres du pavé
  comptent quand le verrouillage numérique est actif ; sans lui, egui reçoit des flèches et le
  code est interrompu (comme sans le crochet). Sur AZERTY, les chiffres de la rangée du haut
  tapés avec Alt retombent sur la touche physique et composent un code eux aussi (hérité). La
  lecture des pages reste dans `mime/src/files/` : elle a besoin des décodeurs de mail-parser,
  et `mime/src/` a déjà ses 16 fichiers.

### État de la récupération : v0.31, première partie : les défauts présents ici (2026-11-02)

L'origine a ajouté 73 commits après `706ca2f2` (jusqu'à `9413d410`, la v0.31). Même méthode, sur la
branche `zidan-v0.31`. Cette partie ne prend que les défauts vérifiés dans le code de ce fork.

- **Repris** : `d03d4805` (le lieu d'une photo retiré en une passe, quels que soient ses décalages,
  trouvé par le fuzz de l'origine) ; `1cda3112` (heures aux extrémités de la plage sans
  dépassement) ; `1852371d` (le premier message d'une Boîte de réception vide compte comme nouveau).
  Ce dernier suppose que `Online` veut dire « courrier entièrement lu » : c'est vrai ici aussi, le
  premier `Online` d'une session n'est dit qu'à la fin de la passe complète (`last_full`).
- **Adapté** : `6ecf53b2` (une synchro de calendrier en attente n'est demandée qu'une fois) : le côté
  réseau était déjà là, seul le moteur manquait, et `settle_answers` passe par le même chemin ;
  `5c4a9d33` (une classification n'est jamais perdue) : ce fork a son propre modèle, sans
  `ical/showing.rs`. Le défaut y était plus large : toute modification renvoyait `Personal` en
  `Normal` et `Confidential` en `Private`, sur les rendez-vous comme sur les tâches.
  `CalEvent::private_kind` et `Task::private_kind` gardent le mot tant que le verrou reste. L'autre
  moitié du commit (un `CLASS` au-delà de 200 lignes) ne s'applique pas : `CLASS` est lu avant le
  plafond. Graph n'envoie toujours pas la sensibilité. Les éléments déjà dans le cache gardent
  l'ancienne lecture tant qu'ils ne changent pas sur le serveur.
- **Corrigé hors de l'origine** : un tableau de rapport dont une cellule vide disparaissait dans les
  rangées sans couleur (les cellules suivantes glissaient sous le mauvais titre) ; voir `c38805d3`.
- **Reste à faire** : `~/mailcraft/merge-tools/survey-v0.31.md` donne les lots suivants et les commits
  laissés.

### État de la récupération : v0.31, deuxième partie : le texte d'abord pour Exchange (2026-11-02)

Le lot 2 de `merge-tools/survey-v0.31.md`, sur la branche `zidan-v0.31` : ouvrir un gros message
d'Exchange (EWS ou Graph) montre son texte tout de suite, le message entier suit. Pris et adaptés :
`59515345` (le message construit à partir de son texte et de la liste de ses fichiers, vides) et
`42c45848` (la même chose pour Graph). Le spec est `docs/superpowers/specs/2026-11-02-exchange-text-first-design.md`.
Rien n'a encore été essayé sur un vrai Exchange : l'origine ne l'a essayé que sur grommunio.

- **Différences avec l'origine** :
  - la liste des champs demandés : `HEADER_FIELDS`, déjà éprouvée par chaque synchro, plus `item:Body`,
    `item:Attachments`, `item:ItemClass` et `item:Importance`, tous de la liste de Microsoft ;
    le `message:InReplyTo` de l'origine n'existe pas chez Microsoft et n'est pas demandé (ni
    In-Reply-To ni References), sans quoi Exchange refuserait la requête entière ;
  - l'expéditeur lu comme la liste le lit (`0x5D01`), pas par un `mailbox()` qui jette toute adresse sans `@` ;
  - le message entier arrive par un fil à lui, sur une copie de la connexion principale, pas sur la
    seconde connexion du compte (sans numérotation ici, et occupée par l'envoi et les salles) ;
  - pas d'en-tête `X-MailCraft-Text-First` : un message reçu aurait pu le porter et rester « en cours »
    pour toujours ; un événement à part, `Event::TextFirst`.
- **Réglé en route** : un nom sans adresse n'est gardé que par `mime::parse_text_first` (le courrier
  ordinaire ne change pas) ; le fil qui trouve déjà le corps en cache envoie `Event::Body` ; seul un
  `NetError::Auth` sur le texte évite le retour au message entier, et une connexion qui échoue répond
  comme `fetch` ; le pool arrête le fil avec le compte ; « Répondre » vérifie le brouillon construit ;
  après un échec, les commandes disent l'échec et non « encore en cours » ; une copie de la connexion
  refusée à la connexion (mot de passe changé dans l'Active Directory, la connexion principale restant
  connectée) ne coûte qu'un refus pour toute la session : les messages en attente sont signalés sans être
  essayés, les suivants viennent entiers par la connexion principale ; la carte d'une invitation attend le
  message entier ; les lignes du journal nomment le dossier et l'uid du message.
- **Vérifié** : les serveurs factices EWS et Graph (dont un qui refuse un champ inconnu comme Exchange),
  le moteur, l'interface. La cible de fuzz `text_first` (les deux constructeurs) est écrite et compile,
  mais n'a pas été lancée : `cargo-fuzz` n'est pas installé ici.
- **Reste à faire** : l'essai sur le vrai serveur du propriétaire (un gros message, un message de dossier
  public ; la ligne du journal en fait foi) ; le courrier plus ancien que ce qui est gardé, lu en ligne,
  ne passe pas par là.

### État de la récupération : v0.31, troisième partie : la recherche dans tous les calendriers (2026-11-03)

La première des quatre fonctions de calendrier de la v0.31 (`merge-tools/survey-v0.31.md`, « Décidé le
02/11 »), sur la branche `calendar-search` : dans le Calendrier, la zone de recherche cherche les
rendez-vous de tous les calendriers cochés (ou d'un seul) et les liste à la place des jours ; un clic va
au jour et ouvre le rendez-vous. Pris et adaptés : `9539f80e` (issue #33 de l'origine) et, pour le lien de
réunion, l'esprit de `54f09098` (les lignes `CONFERENCE` viendront avec la quatrième fonction). Laissé :
`d9135bea` (affichage de droite à gauche), avec le reste de ce travail de l'origine. Spec et plan :
`docs/superpowers/specs/2026-11-02-calendar-search-design.md`, `docs/superpowers/plans/2026-11-02-calendar-search.md`.

- **Différences avec l'origine** :
  - le privé est le `private` du fork et « Afficher comme » son `show_as` (pas le type `Privacy` de
    l'origine, que `5fa58c50` apportait) ; le lien de réunion (`join_url`) est cherché comme un lieu ;
  - la vitesse à l'échelle du propriétaire (25 000 rendez-vous en mémoire quand le calendrier est
    affiché, presque tous avec des notes d'invitation) : chaque champ n'est lu que si le mot y figure, les
    notes en dernier, sans copie des rendez-vous ; « a » sur 25 000 rendez-vous avec notes passe de 2,8 s
    à 0,15 s (compilation de débogage), à résultat identique (un test aléatoire compare l'ancienne et la
    nouvelle façon) ; la liste n'est pas recopiée à chaque image et seules les lignes visibles sont dessinées ;
  - le menu « Tous les calendriers » ne propose que les calendriers dont les rendez-vous sont gardés ici
    (l'origine proposait aussi un calendrier de collègue jamais lu, qui ne trouvait rien) ;
  - Échap vide la recherche où que soit le focus, comme pour le courrier ;
  - le volet latéral du calendrier est passé dans son propre fichier (`panes/calendar/side.rs`) avant
    l'ajout, `calendar.rs` ayant dépassé 800 lignes ;
  - une commande de démarrage, `ui.calendarSearch`, pour voir la recherche dans la vraie fenêtre.
- **Vérifié** : modèle (requête, champs, dates, privé, données absurdes), moteur (une série trouvée une
  fois, à sa prochaine occurrence ou à sa dernière, une occurrence modifiée seule, les annulées et les
  dates retirées écartées, la journée entière à l'est et à l'ouest de UTC, des dates aux limites, 25 000
  rendez-vous réalistes en moins d'une seconde), interface (ouverture au bon jour, Échap, portée devenue
  invalide, données absurdes dessinées), captures de la fenêtre en clair et en sombre.
- **Reste à faire** : l'essai sur le calendrier du propriétaire, avec ceux de ses collègues cochés.

### État de la récupération : v0.31, quatrième partie : l'invitation répondue quitte la Boîte de réception (2026-11-03)

La deuxième des quatre fonctions de calendrier de la v0.31 (`merge-tools/survey-v0.31.md`, « Décidé le
02/11 »), sur la branche `answered-invites`, en comblant les manques plutôt qu'en portant : une invitation
répondue depuis le message partait déjà aux Éléments supprimés, toujours. Maintenant, une réponse donnée
depuis le calendrier (`event.respond`), ou « Supprimer du calendrier » sur une annulation (`invite.apply`),
range aussi le courrier de la réunion, et les versions plus anciennes de l'invitation partent avec, derrière
une case de Fichier › Options › Calendrier (cochée par défaut, comme aujourd'hui). Pris pour référence et
adaptés : `484e4c2f` (issue #31 de l'origine), `25c6997b` (la mise à jour plus récente reste), `3c4febce`
(le message suivant). Choix du propriétaire (03/11) : une seule case, comme Outlook, plutôt que les trois
choix de l'origine (laisser / Éléments supprimés / un dossier par compte) ; l'annulation rangée aussi. Spec et
plan : `docs/superpowers/specs/2026-11-03-answered-invitations-design.md`,
`docs/superpowers/plans/2026-11-03-answered-invitations.md`.

- **Différences avec l'origine** :
  - la recherche des invitations : l'origine lit les sources des 2 000 derniers messages de la Boîte de
    réception (64 Mo) à chaque réponse ; ici, les résumés trient d'abord (objet contenant le titre de la
    réunion, ou envoyé par son organisateur), puis au plus 200 candidats et 16 Mo de sources déjà
    téléchargées sont lus ; rien n'est demandé au serveur ;
  - depuis le calendrier, la version répondue est la copie du calendrier où le rendez-vous a été trouvé
    (pas celle du calendrier partagé d'un collègue, qui peut être en avance), comparée par `SEQUENCE` seul ;
  - Exchange : les rendez-vous lisent enfin leur `SEQUENCE` (`calendar:AppointmentSequenceNumber`, toujours
    0 auparavant), une occurrence modifiée ne recopie pas celle de sa série, et un `MoveItem` dont un
    élément est déjà parti (le serveur range lui-même l'invitation répondue) n'est plus une erreur ;
  - le message suivant est celui qui suit le message lu, pas celui qui suit la plus basse des versions
    rangées ; un message répondu dans les Éléments supprimés y reste ;
  - laissés : les réponses des invités à ses propres réunions (`answered_replies` de l'origine), le
    dossier au choix par compte.
- **Vérifié** : moteur (versions anciennes rangées et plus récente gardée, occurrence seule, réponse
  depuis le calendrier sur IMAP et sur Exchange sans organisateur connu, ses propres réunions laissées,
  copie d'un collègue en avance, annulation, case décochée, Ctrl+Z, message non téléchargé ni lu ni
  demandé, titre trop court, message suivant), faux serveur Exchange (déplacement d'un élément déjà parti,
  version lue), page Options et ses 13 traductions, `cargo xtask ci`.
- **Reste à faire** : l'essai sur le serveur Exchange du propriétaire (répondre depuis le message et depuis
  le calendrier, vérifier qu'aucune erreur « Le serveur a refusé un changement » n'apparaît) ; les
  rendez-vous déjà en cache ne recevront leur `SEQUENCE` qu'à leur prochain changement.

### État de la récupération : v0.31, cinquième partie : privé et « Afficher comme » dans la grille (2026-11-03)

La troisième des quatre fonctions de calendrier de la v0.31 (`merge-tools/survey-v0.31.md`, « Décidé le
02/11 », issue #32 de l'origine), sur la branche `private-show-as`, en comblant les manques plutôt qu'en
portant : le fork avait déjà le privé (`PrivateKind`) et les cinq états d'« Afficher comme », dans la
fenêtre du rendez-vous, sur EWS et sur CalDAV. Avec les deux demandes du propriétaire du 02/11 au soir :
le cadenas dans la grille et un menu au clic droit sur un rendez-vous. Spec et plan :
`docs/superpowers/specs/2026-11-03-private-and-show-as-design.md`,
`docs/superpowers/plans/2026-11-03-private-and-show-as.md`.

- **Ce qui change** :
  - la grille (Jour, Semaine de travail, Semaine, Mois, la bande « toute la journée ») dessine l'état à
    gauche du rendez-vous comme Outlook : occupé plein, provisoire hachuré, libre en contour, absent en
    violet (la couleur de catégorie n° 4), ailleurs en pointillés ; un rendez-vous privé porte le cadenas
    Fluent, et se lit « Rendez-vous privé » quand il arrive sans objet ;
  - le clic droit sur un rendez-vous : Ouvrir, Rejoindre ; Accepter / Provisoire / Refuser pour une
    réunion qu'un autre organise, dans un calendrier à soi ; Afficher comme, Catégoriser et Privé, sans
    prévenir les invités (`event.update` avec `"send": false`) ; Supprimer (cette occurrence ou la série).
    Dans un calendrier en lecture seule, ou sur la copie d'un collègue, seulement Ouvrir et Rejoindre ;
  - une invitation acceptée s'affiche comme l'organisateur l'a voulu (`X-MICROSOFT-CDO-INTENDEDSTATUS`),
    sinon occupée si elle était provisoire (repris de `1ae7e345`) ; la réponse à l'organisateur part sans
    alarme (`6d1ccb3c`, ici sous une autre forme : le rappel partait, et une alarme gardée telle quelle
    n'en perdait que la première ligne) ;
  - Exchange Online (Graph) lit et écrit la sensibilité et l'état (Personnel et Confidentiel gardés).
- **Défauts trouvés en chemin et corrigés** : une modification qui ne touchait pas l'heure donnait le
  fuseau de l'utilisateur à une série qui n'en avait pas, et ses occurrences changeaient d'heure après le
  passage à l'heure d'hiver ; un glisser au bouton droit déplaçait le rendez-vous de 15 minutes ; un
  rendez-vous sur plusieurs jours n'ouvrait aucun menu en vue Mois ; une modification « sans prévenir »
  annulait, sur Exchange, une invitation encore à envoyer (elle serait partie sans destinataire).
- **Laissés** : `5fa58c50` (il remplacerait le modèle du fork) ; `418faa02` (le calendrier par défaut
  choisi de l'origine, #30, non repris : le formulaire du fork part déjà du premier calendrier
  modifiable du compte).
- **Vérifié** : modèle, moteur (invitations sur IMAP, réponse sans alarme, agenda, envoi en attente,
  série au passage à l'heure d'hiver), faux serveur Graph, grille et menu (tests d'interface, clics
  simulés), la vraie fenêtre en clair et en sombre (captures lues), `cargo xtask ci`.
- **Reste à faire** : l'essai sur l'Exchange du propriétaire (EWS) et sur Microsoft 365 (Graph : un état
  choisi depuis le menu, sur une réunion reçue et sur une réunion organisée). Connus, laissés pour
  plus tard : sur EWS, un état ou un privé choisi entre une réponse et la synchronisation suivante est
  perdu (seule la réponse part), et une invitation acceptée s'affiche occupée jusqu'à la lecture suivante
  (`IntendedFreeBusyStatus` n'est pas lu) ; sur Graph, un rendez-vous modifié avant cette version et pas
  encore envoyé part public et occupé.

### État de la récupération : v0.31, sixième partie : la vérification de l'expéditeur (2026-11-04)

Le lot 5 de la tournée v0.31 (`merge-tools/survey-v0.31.md`, issue #29 de l'origine), sur la branche
`sender-check`, adapté au carnet d'adresses du fork plutôt que porté : la partie moteur de l'origine est
bâtie sur un autre carnet. Conçu avec le propriétaire le 04/11 : l'alerte entière, l'adresse après le
nom et l'étiquette Externe en options éteintes au départ, l'annuaire Exchange interrogé à l'ouverture
d'un message, et, son ajout, la signature comparée à l'expéditeur. Spec et plan :
`docs/superpowers/specs/2026-11-04-sender-check-design.md`,
`docs/superpowers/plans/2026-11-04-sender-check.md`.

- **Repris de l'origine** (`pick.sh`, auteur gardé) : `14c0bee4` (le nom d'une personne connue sur
  une autre adresse : `KnownPeople`, `SenderCheck`, les messageries gratuites de chaque pays,
  l'organisation), `22796fda` (lettres stylisées, chiffre pour une lettre, titres), la partie `mime` de
  `f421f17e` (`Parsed::bulk`). La comparaison des noms est accélérée ici sans changer ses résultats.
- **Réécrit** : la partie moteur (`session/opening/senders.rs`). L'origine recompte son carnet à
  chaque synchro de calendrier et garde dix minutes ce qu'elle a compté ; ici les gens connus (comptes,
  fiches de Contacts, correspondants, personnes déjà données par l'annuaire) sont rassemblés quand un
  tampon change (comptes, fiches, révision du carnet, domaines de l'organisation), 45 ms en release
  pour 50 000 personnes. Les marques de la liste viennent sans ouvrir de message (le carnet lu par
  tranches au démarrage), dans la vue en tableau aussi. L'interface reprend `3a87dfa4`, `80fb57d9` et
  `06a623db` dans l'apparence du fork ; les deux réglages de Fichier › Options › Sécurité sont éteints
  au départ (l'origine montre d'emblée l'adresse des inconnus).
- **Ajouté ici** :
  - la question à l'annuaire Exchange quand on ouvre le message d'un inconnu au nom de personne : une
    fois par nom, la dernière en attente remplace la précédente, jamais devant Vérifier les noms ; une
    personne connue seulement par l'annuaire n'alerte que depuis une messagerie gratuite ou un domaine
    sosie (les homonymes d'une grande liste d'adresses) ;
  - la règle de signature (`lookalike/signature.rs`, cible de fuzz `signature`) : le texte propre à
    l'auteur jusqu'à la première marque de citation (blocs d'en-tête d'Outlook dans chaque langue,
    intitulés en gras de la conversion HTML, lignes de Gmail, transferts d'Apple), les seules lignes
    courtes de sa fin, et trois cas : la signature de l'organisation venue d'ailleurs, une adresse connue
    venue d'une autre, le nom de l'expéditeur sur une adresse de messagerie gratuite ;
  - les fournisseurs d'accès français parmi les messageries (wanadoo.fr, laposte.net, sfr.fr…).
- **Corrigé en chemin** (préexistant) : une question de Vérifier les noms posée sur un nom déjà en
  attente pouvait être évincée par le nom tapé suivant, et la fenêtre attendait une réponse qui ne
  venait pas.
- **Laissés** : les parties arabes de l'origine ; son cache de dix minutes.
- **Vérifié** : modèle, moteur et interface (tests sans fenêtre : bandeaux, liste, tableau, Options),
  la vraie fenêtre (liste avec adresses et Externe, vue en tableau ; captures lues), le temps du
  recalcul en release, `cargo xtask ci`.
- **Reste à faire** : l'essai sur la boîte du propriétaire (un faux collègue sur Gmail, la question à
  l'annuaire, le temps du premier démarrage). Laissés pour plus tard : retenir les marques de la liste
  par message plutôt que de les recalculer à chaque image ; le lecteur de signature ne voit pas les
  notes de liens d'une réponse citée ; un domaine d'organisation refusé dans Options l'est sans
  message ; les noms écrits de droite à gauche ne sont pas alignés à droite dans les lignes marquées.
  Plus tard (1.2) : un bouton « Est-ce une arnaque ? » de l'assistant, à la demande, qui ne pourra
  qu'ajouter une mise en garde.

### État de la récupération : v0.31, septième partie : plusieurs salles Webex et CONFERENCE (2026-11-04)

Fonction n° 4 du calendrier de ZIDAN44 (v0.31, issue #25 de l'origine), sur la branche `webex-rooms`,
conçue avec le propriétaire le 04/11 : plusieurs salles Webex, chacune avec son nom, son lien et des
lignes libres, et la lecture et l'écriture de `CONFERENCE` (RFC 7986). Spec et plan :
`docs/superpowers/specs/2026-11-04-webex-rooms-conference-design.md`,
`docs/superpowers/plans/2026-11-04-webex-rooms-conference.md`. Mode d'emploi : `docs/webex.md`.

- **Repris de l'origine** : la lecture de `CONFERENCE` de `e838af15`, réécrite ici (premier lien web
  dont `FEATURE` n'est pas seulement téléphone, discussion ou flux ; les autres lignes gardées et
  réécrites telles quelles ; liens et étiquettes bornés ; fonctions de texte iCalendar rangées dans
  `ical/text.rs`).
- **Ajouté ici** :
  - plusieurs salles (Fichier › Options › Calendrier, « Salles Webex » : nom de 80 caractères, lien
    de salle personnelle, jusqu'à 20 lignes libres de 200 caractères ; 20 salles au plus, liens
    uniques), la salle unique de la 1.0 reprise comme première salle, sans nom, affichée « Ma salle
    personnelle » ; commande `calendar.webexRoom` ;
  - le menu du bouton Réunion Webex : « Nouvelle réunion Webex », les salles par nom, « Ajouter une
    salle… » ; les salles grisées tant qu'une réunion Webex est planifiée ; dans le navigateur, un
    clic avec une seule salle l'ajoute ;
  - le remplacement du bloc d'une salle : « Rejoindre la réunion Webex : », le lien et les lignes en
    fin de notes ; une autre salle (ou une réunion planifiée) remplace ce bloc, retrouvé dans toutes
    les langues de l'interface, les lignes tapées par l'utilisateur restant ;
  - la règle de retrait : une conférence dont le lien est effacé du lieu et des notes est retirée ;
    celle d'une autre application reste ;
  - l'écriture partout où c'est possible : CalDAV, calendriers locaux, invitations envoyées depuis un
    compte IMAP, export `.ics` (`CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Webex:<lien>`), rien
    vers Exchange (EWS, Graph) ; Rejoindre prend le lien du serveur, puis la conférence, puis un lien de
    réunion connu du lieu ou des notes ; la recherche trouve l'étiquette et le lien ;
  - Rejoindre et Réunion Webex côte à côte dans le ruban d'un rendez-vous modifiable ;
  - `event.create` et `event.update` acceptent `conference`.
- **Laissés** : les salles d'autres services (BigBlueButton, Jitsi, OpenTalk), son bloc
  `---- Online meeting ----` à marqueurs, la liste des organisateurs de réunions, « ajouter une réunion
  en ligne à toutes les réunions avec invités », sa carte Réunions en ligne, ses commandes `meetings.*`,
  Teams par Graph.
- **Vérifié** : modèle, moteur (aller-retour CalDAV, invitation IMAP, rien vers Exchange), interface
  sans fenêtre (menu, remplacement du bloc, retrait, lignes d'Options), cible de fuzz `ical` étendue,
  `cargo xtask ci`.

### État de la récupération : huitième partie : la mise en page HTML (2026-11-04)

Le lot 3 de `merge-tools/survey-v0.31.md` (`223e1962`, `7ae3b393`, v0.31) et ses deux suites de
l'issue #41 de l'origine, arrivées après la v0.31 (`b06767f4`, `d153a457`, 04/11), sur la branche
`lot3-mise-en-page` ; un commit par commit de l'origine, à son nom. Avant : un courrier en une
colonne centrée de largeur fixe (lettre d'information de 600 px, message écrit par Outlook en
`width:450pt`, `max-width:37.5em` des outils d'e-mailing, largeur posée sur `<body>`) n'avait aucune
mise en page ici et s'étalait sur toute la largeur du volet de lecture ; 10 des 13 cas de test de
l'origine échouaient.

- **Repris** : une boîte ou un tableau d'une largeur fixe garde sa largeur, centré là où le message le
  centre ; `max-width` borne un `width: 100%` ; les unités pt, pc, in, cm, mm, em et rem pour les
  colonnes et les tailles d'images ; `auto` ne fixe rien, une unité illisible laisse l'attribut
  `width` ; un `<body>` de largeur fixe est une colonne ; dans un courrier mis en page, un paragraphe
  centré est une cellule centrée, et une taille posée dans `<b><span style="font-size:20pt">` en fait
  un titre, comme Outlook l'affiche.
- **Adapté** :
  - la photo d'une carte de visite : la cellule d'une seule image prend le `fit` qu'un logo flottant
    avait déjà (largeur de l'image, marge ajoutée au dessin) ; la règle d'empilement du volet de
    lecture de ce fork garde déjà une colonne étroite demandée par le message, le changement de
    l'origine sur l'empilement n'est pas pris ;
  - le centrage d'un tableau dans une cellule centrée existait déjà ici, et plus largement
    (`reading/body/table.rs`) : la partie de `223e1962` sur le volet de lecture n'est pas prise ;
  - les unités passent par le `length_px` des bordures et marges de ce fork, qui reconnaît désormais
    l'unité par la fin du texte : un nombre écrit avec exposant (`1e9`) reste un nombre, comme les
    tailles d'images le lisaient avant ;
  - les largeurs vont dans `layout/lengths.rs`, les titres dans `layout/styling.rs`.
- **Vérifié** : les tests de l'origine adaptés, ceux du fork (signatures d'Outlook côte à côte,
  cellules vides, images cliquables, bordures), le moteur, l'interface sans fenêtre ; dans la vraie
  fenêtre, quatre courriers d'essai avant et après, et la lettre d'information de la boîte d'exemple,
  identique au pixel près.
- **Reste à faire** : l'essai du propriétaire sur ses vraies lettres d'information ; les boutons
  dessinés par la marge interne d'un lien (carnet `lecture-html-boutons`) ne sont pas dans ce lot.

### État de la récupération : v0.32, neuvième partie : plantages, entrées hostiles et certificats du système (2026-11-04)

Les lots R1 et R1b de `merge-tools/survey-v0.32.md`, sur `main`. Pour chaque commit, son test porté
seul d'abord et lancé chez nous : la correction n'est prise que si le test échouait. Les reprises
telles quelles sont au nom de leur auteur, les adaptations au nom du propriétaire.

- **Repris** :
  - `37170bcd` : une réponse IMAP imbriquée à plus de 64 niveaux est refusée avant d'être analysée.
    Sonde : sur une pile de 1 Mio, débordement de pile et arrêt du processus (SIGABRT) dès
    1 000 niveaux : un serveur hostile fermait Quatrelle.
  - `215cca05` : async-executor 1.14.1, la 1.14.0 étant retirée de crates.io. Sonde : `cargo deny`
    disait d'abord tout bon, faute d'index à jour ; index rafraîchi, il refusait la 1.14.0.
  - `900162b8` : les tests d'instance unique cessent d'attendre une réponse sans fin. Pas de sonde
    possible (le blocage vient d'une fermeture que Windows perd sous charge) ; ne touche que les tests.
  - `28ad403a` : les serveurs dont le certificat vient d'une autorité installée sur le système se
    connectent (IMAP, POP, Sieve, HTTPS, SMTP), en plus des racines Mozilla ; les certificats que
    Windows désavoue (`Disallowed`) restent refusés ; une racine illisible du magasin n'empêche plus
    d'envoyer. Décision du propriétaire du 04/11. Sonde : l'autorité de test posée comme celle du
    système (`SSL_CERT_FILE`), la configuration TLS du fork refusait le serveur (`UnknownIssuer`) ;
    elle l'accepte après. `schannel` (MIT, Windows) devient une dépendance directe ; il était déjà
    dans l'arbre. La règle de `CLAUDE.md` le dit ; `docs/gaps.md` (n° 8) note ce que SMTP ne peut
    pas refuser (lettre ne prend pas de vérificateur).
- **Adapté** :
  - `dee9fff3` (vCard) : sous le plafond de 1 Mio du fork, la lecture quadratique prenait 4,6 s, sous
    la borne de 30 s du test de l'origine ; la borne est ici d'une seconde. Corrigé en même temps :
    une coupe du plafond au milieu d'un caractère lisait tout le texte (`get()` échouait) ; l'origine
    l'avait déjà corrigé.
  - `b38b8e2a` puis `9157dc1e` (pages de code de winmail.dat) : le lecteur TNEF du fork est dans
    `attachments/tnef`, `codepage.rs` y remplace son `decode` ; la table est publique pour l'import
    .pst à venir. Sondes : 24 des 46 pages du test de l'origine étaient mal lues (ISO-2022-JP,
    UTF-7, UTF-16BE, EUC-JP, plusieurs ISO-8859, les cinq octets non définis de Windows-1252) ; un
    RTF en `\ansicpg1200` perdait tout son texte, un texte 8 bits étiqueté 1200 devenait des
    idéogrammes.
  - `e023a6e9` (trousseau) : le fork garde `Keychain` et ses tests Windows dans
    `desktop/keychain.rs` ; ils y reçoivent leur nettoyage et le contrôle des morceaux orphelins.
    Non sondé : ces tests ne tournent que sous Windows, et la vérification de types pour Windows
    n'est pas possible sur ce poste (`ring` demande l'outillage MSVC). Sous Linux et macOS, seul
    changement : les appels au trousseau passent un par un.
- **Écarté** : rien ; aucune sonde n'est passée avant la correction.
- **Vérifié** : chaque test porté, les tests des crates touchés, la sonde des certificats refaite
  après la reprise, `cargo deny check`, `cargo xtask ci`.
- **Reste à faire** : les tests du trousseau et des certificats sur un vrai Windows (avec une
  autorité interne, comme l'Exchange du propriétaire) ; la suite du relevé, à commencer par P1 (le
  lecteur .pst corrigé) puis R2.

### État de la récupération : v0.32, dixième partie : le lecteur .pst corrigé (2026-11-05)

Le lot P1 de `merge-tools/survey-v0.32.md` (décision n° 1 du propriétaire, 04/11), sur `main`.
Les tests de l'origine portés d'abord et lancés un par un contre le lecteur d'alors, puis le
changement de crate.

- **D'où vient le crate** : `outlook-pst-rw` 1.2.14 (Dong Shin, MIT : le lecteur de Microsoft,
  `outlook-pst`, avec un rédacteur ; sa bibliothèque garde le nom `outlook_pst`), construit depuis
  [dgaudin/outlook-pst-rs](https://github.com/dgaudin/outlook-pst-rs), dépôt indépendant du
  propriétaire (pas une fourche GitHub), copie de la branche `craft/rw-1.2.14` de la fourche de
  MailCraft (ZIDAN44/outlook-pst-rs), à la révision `60cefd986b159d5f934d8291e4317dc96e0cd35b`,
  celle qu'épingle l'origine. Son `PATCHES.md` liste les correctifs et dit comment monter de
  version. Épinglé par révision dans `[patch.crates-io]` de `Cargo.toml` et de `fuzz/Cargo.toml`,
  admis dans `allow-git` de `deny.toml` ; le dépôt de ZIDAN44 n'est nommé nulle part. Un seul
  crate lit et écrit : `outlook-pst` 1.2.0 de crates.io et le renommage `pst_rw` disparaissent.
- **Sondes**, contre `outlook-pst` 1.2.0 (le rédacteur `outlook-pst-rw` 1.2.14 de crates.io
  fabrique les fichiers, abîmés ensuite octet par octet ; deux méthodes absentes de la 1.2.0,
  `set_max_value_size` et `embedded_message`, remplacées le temps de la sonde par des méthodes
  vides) :
  - `a_data_tree_that_points_at_itself_is_an_error` (`2db969b0`) et
    `a_btree_page_that_points_at_itself_is_an_error` (`e3b7c919`) : débordement de pile, « fatal
    runtime error: stack overflow », SIGABRT en 0,3 s : tout le binaire de test meurt. Par
    `pst::import` (test ajouté, `the_import_of_a_tree_that_points_at_itself_ends`) : pareil, le
    `catch_unwind` de l'import n'y peut rien ; un tel fichier fermait Quatrelle.
  - `a_data_tree_that_names_one_block_over_and_over_reads_no_more_than_it_declares`
    (`e3b7c919`) : 24 528 octets rendus pour 20 000 déclarés. Poussé plus loin (test ajouté,
    `a_data_tree_that_fans_out_over_one_block_is_held_to_its_size_and_the_limit` : un XXBLOCK de
    1 021 entrées vers un XBLOCK de 1 021 entrées vers un bloc de 3 648 octets) : un fichier de
    525 Ko fait monter le lecteur à 2,1 Go résidents, arrêté seulement par un `ulimit -v` de 3 Go
    (« out of memory ») ; sans plafond il visait 3,8 Go (pas lancé : 5 Go libres sur le poste).
  - `a_data_tree_reads_no_more_than_it_declares_or_is_allowed` (`2db969b0`) : la lecture entière
    passe, la 1.2.0 n'a pas de plafond, et un XBLOCK qui déclare 100 octets en rend 20 000.
    `a_store_or_name_map_value_is_held_to_the_file_limit` (`23d38bbb`) : lu sans plafond.
  - Surprise : la 1.2.0 ne réserve rien d'après la taille déclarée (un XBLOCK qui déclare 4 Gio
    rend ses 20 000 octets avec 21 Mo résidents). L'allocation de 4 Gio que décrit `23d38bbb`
    venait de la première version corrigée de l'origine, qui lit jusqu'à la taille déclarée ; le
    plafond reste nécessaire avec le lecteur corrigé, dont la limite par défaut est `u64::MAX`.
  - `an_attached_message_and_an_empty_file_read` (`cd52356e`) : erreur `AttachmentMethodNotFound`
    (l'enregistrement `PtypObject` arrêtait la lecture des propriétés de la pièce) ;
    `an_attached_message_in_the_message_tree_reads_when_its_own_tree_is_damaged` (`e3b7c919`) :
    « Missing PidTagAttachMethod » ; `the_import_keeps_an_attached_message_and_an_empty_file`
    (`cd52356e`) : le message entre, sans son message joint ni son fichier vide, perdus en silence.
  - Aucun test ne passait avant ; tous passent après, sous 25 Mo résidents chacun.
- **Adapté** (au nom du propriétaire, en un commit pour le crate et un pour le fuzz) :
  - `3bcf229e`, `44b511b6`, `0cb9196d` : le crate épinglé directement à la révision finale ;
    `cargo xtask ci` vérifie que les deux manifestes l'épinglent pareil (`FORKED`, `patched`).
    Les `use` de `pst.rs`, `pst_export.rs`, `pst_bench.rs` et des tests passent à `outlook_pst` ;
    le rédacteur prend désormais `message_class` et `properties` (l'export écrit `IPM.Note` sans
    propriété de plus, comme avant ; l'export fidèle est P4).
  - `2db969b0`, `cd52356e`, `e3b7c919`, `23d38bbb` : `pst/tests/reader.rs` (état de `23d38bbb`,
    plus les deux tests des sondes) ; la macro `reader!` de l'import pose un plafond de 256 Mo par
    valeur (`MAX_VALUE`), pour les fichiers Unicode et ANSI. La part de `23d38bbb` dans
    `pst_export.rs` n'a pas d'objet ici : l'export du fork écrit toujours un fichier neuf et ne
    lit aucun fichier existant (le rédacteur relit le sien sous son propre plafond de 256 Mo).
  - `bd464a49` : la cible de fuzz `pst` compte le crate construit depuis git
    (`.cargo/git/checkouts/outlook-pst-rs-…`) comme code tiers, et prend la graine
    `seeds/pst/attached-message` de l'origine.
- **Écarté** : les épingles intermédiaires de l'origine (branche `craft/1.2.0`), la découpe de
  `pst.rs` et le reste de l'import de l'origine (lots P2 et P4).
- **Vérifié** : les dix tests du lecteur un par un, la suite de `quatrelle-net` (import et export
  compris), `cargo xtask ci`, `cargo deny check` (index rafraîchi), et la cible de fuzz `pst`
  vérifiée par `cargo check` dans `fuzz/` (`cargo-fuzz` n'est pas installé sur ce poste : pas de
  `cargo +nightly fuzz build pst`).
- **Reste à faire** : les fichiers d'un message joint (le lecteur les ouvre maintenant, l'import
  ne les prend pas encore : P2), une passe de fuzz sur la cible `pst` avec la nouvelle graine,
  puis R2.

### État de la récupération : v0.32, onzième partie : ne rien perdre sur une erreur du cache (2026-11-05)

Le lot R2 de `merge-tools/survey-v0.32.md` (constats n° 4 et 7), sur `main`. Pour chaque commit,
son test porté seul d'abord et lancé contre le code d'alors : la correction n'est prise que si le
test échouait. Le stockage garde le scellement à chaque commit du fork (`38594552`) et son
compactage en place : rien de la machinerie de commits non durables de l'origine (`652ac8f6`,
`packed/commit.rs`, `Seals`, `open.rs`).

- **Refait à la main** : `85f5c255` (`1e317bfc`), pur déplacement : `Files` et `Routed` de
  `lib.rs` vers `files.rs` (identiques à l'origine, vérifié par `diff`), tables, compression,
  pièces et `read_file` de `packed.rs` vers `packed/kept.rs`, avec nos préfixes hérités
  `DEFLATED_OLD`/`PIECED_OLD` ; `PIECED` est `pub(super)` pour notre test des noms hérités. Les
  30 tests de `quatrelle-store` passent avant et après.
- **Repris tels quels** (au nom de leur auteur) :
  - `4d3359dd` (`6c61ccd8`) : un brouillon gardé hors ligne ne réécrit plus une liste
    `drafts-pending.json` illisible. Sonde : la liste réécrite avec le seul dernier brouillon.
  - `43f36a6a` (`81f4fb78`) : POP, un relevé des messages téléchargés illisible arrête le
    relevé ; un dossier illisible n'a plus de comptes à 0. Sonde (sans ses deux lignes sur
    `local_counts`, qui ne compilent pas avec l'ancienne signature) : le relevé continuait.
  - `756374e8` (`ca8ba64f`) : POP, un déplacement classe d'abord dans la destination, puis
    enregistre les sources, puis ôte les anciennes sources ; numéros contrôlés. Sonde : sources
    illisibles, le déplacement se disait fait et les messages arrivaient sans leur source.
- **Adapté** (au nom du propriétaire) :
  - `2cebf2c6` (`d83b5b16`) : `Store::failing` et `Store::fail` (un cache en mémoire dont les
    lectures ou écritures échouent sur demande), `load_body` (source absente ≠ erreur). Sans le
    compteur `reads` de l'origine ; `data_dir` du fork prend la nouvelle variante. Sonde : le
    test échouait d'abord sur `add_op` qui réécrivait un `ops.json` illisible, puis, cela
    corrigé, sur la source qu'un Expunge ôtait alors que la liste n'avait pu être enregistrée
    (EmptyFolder corrigé de même).
  - `2f92bc45` (`907a3e4e`) : source abîmée lue comme absente, `free_uid`/`take_uid`. Ici
    `free_uid` sert tout `FolderState` (la session garde ses listes partagées). Pas de test
    propre : ses sondes sont celles de `f8e6dbfd`.
  - `01f49293` (`bf19ef67`) : le déplacement local lit toutes les listes avant d'écrire et
    s'arrête sur une erreur du cache (`file_moved`) ; `file_locally` rend une erreur ; la
    nouvelle copie d'un brouillon est classée avant que les anciennes partent (ici dans
    `file_draft`, que partagent l'enregistrement et les brouillons gardés à la fermeture) ;
    un agenda illisible n'est ni réécrit ni rangé (le rangement des rendez-vous des autres du
    fork, `parking.rs`, réécrit aussi `pim.json`) ; une liste illisible reste hors du cache de
    la session et garde ses comptes. Sondes (trois tests de l'origine) : la destination vidée
    (ses deux messages perdus), le brouillon dit enregistré et le message dit envoyé avec
    Brouillons illisible, aucun avis pour un agenda illisible.
  - `f8e6dbfd` (`74a19d45`) : un déplacement raté échoue sans pas d'annulation
    (`apply_op_checked`) ; numéros pris par `free_uid`/`take_uid` ; `rules.json` et
    `snoozes.json` illisibles jamais réécrits (lus dans `organize.rs`, et le second écrivain
    de `rules.json` du fork, `save_rules`, retenu aussi) ; `pim_unread` oublié au retrait du
    compte. Avant, `organize.rs` (839 lignes) est découpé : les règles vont dans
    `organize/rules.rs` (`6184ff91`, pur déplacement). Sondes : le déplacement raté répondait
    `{"moved": 1}`, une destination à `u32::MAX` prenait le message (l'ancien `+ 1` débordait :
    panique en debug, 0 en release), aucun avis pour `rules.json` illisible, le compte retiré
    restait marqué. `a_damaged_source_moves_without_it` passait déjà : `2f92bc45` fait ce
    travail, et l'ancien code du fork (`read_body().ok()`) aussi.
  - `690c6214` (`ac4f921f`) : les modifications en attente d'un compte retiré sont abandonnées
    (`Store::removals`, `Store::while_kept`), dans l'écrivain du fork (`write_edit` de
    `worker.rs` ; l'origine a `worker/edits.rs`). Sonde, à 300 marques plutôt que 2 000 (au-delà
    de sept minutes ici, voir plus bas) : 140 836 octets du compte supprimé réécrits.
  - Avis nouveaux dans les treize catalogues (traductions de l'origine, le français et
    l'allemand au registre du fork) ; « Your mail couldn't be moved… » y était déjà, sans
    code qui s'en serve.
- **Propre au fork** : `3152956f`, un commit vide scellé (`set_quick_repair(true)`) après
  `Packs::compact` : les commits du compactage de redb n'enregistrent pas l'état de
  l'allocateur. Sonde (`a_data_file_killed_after_compacting_needs_no_repair`, sur le modèle de
  `a_data_file_left_by_a_crash_needs_no_repair`) : un enfant qui compacte puis s'arrête net
  laissait un fichier à vérifier en entier ; il rouvre sans réparation après.
- **Écarté** : la machinerie de commits non durables de l'origine ; rien d'autre, aucune
  correction prise sans que son test échoue d'abord.
- **Vérifié** : chaque sonde avant et après, les suites de `quatrelle-store`, `quatrelle-engine`,
  `quatrelle-net` et les tests i18n, `cargo xtask ci`.
- **Reste à faire** : un arrêt brutal *pendant* le compactage demande toujours une vérification
  complète (redb ne peut pas sceller en cours de compactage) ; chaque enregistrement d'une liste
  rejoue toutes les modifications en attente (`pending_edits`), si bien qu'une rafale de
  marques hors ligne coûte au carré (2 000 marques sur un dossier de 2 000 messages dépassaient
  sept minutes en debug) ; les rendez-vous rangés (`pim-parked.json`) et l'agenda relu après un
  import se lisent encore « vide si illisible » (copies du serveur, relues à l'affichage) ; la
  suite du relevé, R3.

### État de la récupération : v0.32, douzième partie : compression du cache et import qui ne perd pas son travail (2026-11-05)

Les lots R3 et R4 de `merge-tools/survey-v0.32.md` (constat n° 1), sur `main`. Pour chaque
commit, son test porté seul d'abord et lancé contre le code d'alors ; quand l'origine n'en avait
pas, ou que le sien ne montrait pas le défaut (la mémoire), une sonde écrite ici.

- **R3, compression et mémoire du cache** :
  - `3ecd128a` (`8f1e2198`, repris tel quel, `DEFLATED_OLD` gardé) : `pack()` compresse avec
    miniz_oxide (niveau 1) appelé directement. Sonde : le test d'intégration de l'origine, porté
    en `apps/quatrelle/tests/data_file_compression.rs`, échouait : dans le graphe de
    l'application, `hayro-syntax` fait prendre zlib-rs à flate2, dont le niveau rapide gardait le
    base64 d'une pièce jointe à **0,996** de sa taille, donc stocké brut (le test du crate seul
    passait). Après : **0,760** (une source de base64 tient dans 0,733 de la place prise par du
    bruit). Le format ne change pas : anciens fichiers et anciennes versions se relisent.
  - `9c0e3e5e`, `239a8625`, `104fc902` (tampon écrit en place, puis sa taille) : **non pris**.
    Leurs tests passaient déjà (`21db2772`, gardés comme garde-fous) : notre `pack()` garde
    exactement ce que dit la règle du dixième, dans un tampon de cette taille, et rien ici ne
    garde une source compressée en attente dans un lot d'import (le `652ac8f6` de l'origine, que
    ces commits corrigent, n'est pas pris). Reste, non mesuré par leurs tests : `compress_to_vec`
    double son tampon (pic passager d'environ 1,76 fois la source pendant la compression).
  - `1a8bed0c` (`e685c242`, adapté) : un fichier compressé en pièces est décompressé au fil de
    leur lecture, sans recoller les pièces d'abord ; l'ancien préfixe est lu aussi. Le test de
    l'origine (pièces manquantes détectées) passait avant et après. Sonde écrite ici,
    `store/tests/reading_memory.rs` (Linux, pic de mémoire résidente) : une source de 64,5 Mo
    montait à **1,75** fois sa taille au-dessus de l'état d'avant ; **1,00** après. Le test des
    préfixes hérités couvre maintenant une valeur compressée en pièces.
- **R4, un import qui ne perd pas son travail** :
  - `4f130900`, seulement `RESERVE_MAX` (`65d4aba4`) : 64 Mio au plus réservés avant de lire un
    fichier, au lieu de 1 Gio sur la foi de sa note. Sonde (même fichier) : une note annonçant
    1 Gio sans aucune pièce faisait monter la mémoire virtuelle de 1 006 649 344 octets ; 16 Ko
    après. La cible de fuzz `data_file` n'est pas prise (elle suppose le stockage de l'origine).
  - `e44b556b` (`6dcbcc80`, repris tel quel, dans `worker/pool.rs`) : un fil d'import qui ne
    démarre pas termine l'import en échec au lieu d'un simple avis (compte laissé en
    « Syncing »). Pas de sonde possible (pas d'`unsafe` ni de `setrlimit` pour empêcher un fil).
  - `0a59342b` (`fdcdac9b`, adapté) : si l'enregistrement final (liste des dossiers, ou contacts,
    calendrier, notes et tâches) échoue, `finish` rend les comptes avec l'erreur
    (`Imported::last_save`, qui dit lequel) ; le moteur garde le compte quand du courrier ou un
    élément est entré (lu dans le cache : l'événement du fork n'a pas de rapport), réenregistre
    la liste des dossiers et le dit ; si cela échoue encore, il dit de réimporter. Deux avis
    nouveaux dans les treize catalogues. Sondes (`pst/tests/losses.rs`,
    `engine/src/tests/accounts/data_files.rs`) : `finish` rendait
    `Err("memory/d1/folders.json: failed on purpose")` sans comptes ; un compte de 30 messages et
    un de contacts seuls étaient supprimés. Non pris : `ImportReport`, la progression en éléments
    lus, les dossiers écartés (journal, RSS) : lot P2.
  - `cda89c0f`, seulement le `catch_unwind` par dossier (`1e7f03c9`, adapté) : `visit` lit chaque
    dossier (`visit_one`) et sa liste de sous-dossiers derrière leur propre `catch_unwind` ; ce
    qui est entré du dossier fautif reste, ses sous-dossiers sont lus. Sonde (point de panne de
    test `pst::tests::fault`) : une panique dans le premier de trois dossiers arrêtait l'import,
    les deux autres perdus. Pour rester sous 800 lignes, le test ignoré `real_files` passe dans
    `pst/tests/real_files.rs`. Ni le pipeline ni la découpe de `pst.rs` de l'origine.
  - `e70504df` (`e21d6e32`, repris tel quel) : l'export remplace l'ancien `.pst` par un seul
    renommage ; s'il échoue, l'erreur dit que le courrier est dans le `.pst.part`. Sonde : le
    test de l'origine (une cible qu'on ne peut remplacer) échouait, l'erreur n'étant que « Is a
    directory ». La perte elle-même (suppression réussie, renommage raté) ne se provoque pas ici.
  - `2071a661` (`fd3bbfd1`, adapté) : aucun `vendor/` suivi par git ; le filtre disparaît de
    `xtask sizes` et aussi de `xtask authors` (propre au fork). La partie `keys` est sans objet :
    ses commentaires ne parlent pas de code tiers copié dans le dépôt.
- **Vérifié** : chaque sonde avant et après, les suites de `quatrelle-store`, `quatrelle-engine`,
  `quatrelle-net` et les tests i18n, `cargo xtask ci`.
- **Ce que R3 ne rend pas** : la place déjà prise par les pièces jointes gardées brutes dans les
  fichiers de données existants ne revient qu'avec la recompression (lot R12) ; seules les
  écritures nouvelles sont compressées.
- **Reste à faire** : la suite du relevé, R5.

### État de la récupération : v0.32, treizième partie : le courrier qui se dit envoyé sans partir, et les fichiers joints altérés (2026-11-05)

Le lot R5 de `merge-tools/survey-v0.32.md`, sur `main`. Pour chaque commit, son test porté seul
d'abord et lancé contre le code d'alors.

- **Envoyer seulement depuis un compte qui le peut** :
  - `40f0856d` et `71f43fa5` (`1086c6ea`, adaptés ensemble) : `Session::send_block` dit pourquoi
    un compte ne peut pas envoyer (aucun compte, un fichier de données, une boîte gardée ici
    seulement, pas de serveur sortant) ; `compose.send` est éteint avec cette raison, et
    `deliver`, Envoyer maintenant et la minuterie de la boîte d'envoi, la réponse à une
    invitation refusent aussi. Un nouveau message, une réponse ou un transfert dans un fichier de
    données part du premier compte qui peut envoyer ; De ne propose que ceux-là ; un compte
    ajouté prend les messages en cours d'un compte qui ne peut pas, avec sa signature (la
    signature unique par compte du fork : `put_signature` remplace l'ancienne là où elle est, ou
    se place comme au début d'un message). Sondes : les cinq tests de l'origine
    (`tests/writing/sending.rs`) échouaient ; écrit depuis un `.pst` seul, `compose.send`
    rendait `{"sent": …, "local": true}`, classait le message dans « Éléments envoyés » et disait
    « Message sent ». Sans les changements de `71f43fa5`, ses deux tests échouaient aussi. Test
    de la ligne de commande porté (`apps/quatrelle-cli/tests/sending.rs`).
  - **La barre** est celle du fork, pas celle de l'origine : le cadre teinté des avertissements
    du volet de lecture (vérification de l'expéditeur, sécurité ; `t.warn` à 0,12, coins 6),
    l'icône d'avertissement, ce qui ne va pas en `t.text` (« Impossible d'envoyer du courrier
    depuis un fichier de données Outlook. »), dessous en petit ce qu'il faut faire (« Ajoutez un
    compte pour envoyer du courrier. ») et le lien « Ajouter un compte », comme « Afficher les
    images » de la barre des images. Le bouton Envoyer est dessiné éteint, sa bulle dit pourquoi,
    un clic l'explique dans l'alerte du message, qui offre aussi Ajouter un compte et ne se
    ferme plus sous le Ctrl+Entrée qui l'a ouverte. Textes découpés dans ceux de l'origine (cinq
    chaînes, treize catalogues) ; l'avis de l'invitation non envoyée est un gabarit traduit.
    Vérifiée sur les images sans écran : `tests/writing/sending.rs` de l'interface (place de la
    barre au-dessus du formulaire, note et lien sur une ligne, bouton éteint, barre disparue une
    fois le compte ajouté) et un cas ajouté au tour (`tests.rs::tour`, aussi en pseudo-langue).
- **Les fichiers joints tels qu'ils sont venus** :
  - `48279403` (`8271e18e`, repris tel quel) : `attachments_data`, plusieurs pièces d'une seule
    analyse. Le test ne compilait pas avant (fonction absente). Le volet de lecture analyse encore
    le message une fois par image (`reading/pictures.rs`) : l'origine ne s'en sert qu'à `321bef22`
    (lot R11).
  - `21b6d775` (`945dcd10`, repris tel quel) : un fichier texte est repris de la source, seul
    l'encodage de transfert défait. Sonde : un fichier en Windows-1252 était enregistré en UTF-8
    (`c3 a9` pour « é »), un fichier sans `charset` avec `U+FFFD` à la place de ses accents ;
    l'octet `0x80` devenait « € » (graine proptest gardée). L'aperçu de `attachment.preview` lit
    le texte dans son jeu de caractères (`attachment_text`).
  - `c15d330d` (`bfe2b57c`, repris tel quel) : l'invitation lue dans son jeu de caractères.
    Sonde : après `21b6d775`, « Caf� review » (test dans `tests/invites/charset.rs`).
  - `9ada36cb` (`ed72d5e4`, repris tel quel) : l'aperçu du volet de lecture, de même. Sonde :
    « Caf� au lait » (test dans `tests/reading/attachment_text.rs`).
  - `39f3547e` (`b288b8e8`, adapté) : Transférer et une boîte d'envoi reprise gardent le
    `charset` (`attachment_out`) ; ici aussi un brouillon rouvert, qui avait le même défaut.
    Sondes : le transfert et le brouillon partaient en `text/plain` nu.
- **`86ba6746`** (`8d07a07a`, adapté) : une image en ligne nommée part avec `filename=` et
  `name=`. Sonde : pas de `filename=`. Ici une partie nommée comptait toujours comme un fichier
  (la règle de l'origine, qui lit les références du HTML, est celle du lot R6) : nommer nos
  images aurait mis un trombone sur chaque message envoyé avec l'image d'une signature. Règle
  étroite à la place : une image envoyée en ligne avec un `Content-ID` que le HTML du message
  montre par `cid:` est au texte, pas un fichier — **les logos `image001.png` d'Outlook reçus par
  IMAP perdent donc aussi leur trombone** ; un fichier envoyé en ligne (les PDF d'Apple Mail)
  reste un fichier. Ni `draft_carries_files` ni `index_doc` ici : le trombone du test se vérifie
  par `summary_from_full` ; le test `.pst` est écrit sur `to_raw`.
- **`3aeaae69`** (`9f1e170e`, adapté) : `build` emprunte les pièces du brouillon. Le garde-fou
  passait avant et après, avec les empreintes de l'origine prises après le nommage des images
  (les mêmes ici) ; sa longueur laisse de côté la ligne `X-Mailer` comme son empreinte (la
  version diffère d'un caractère). Mesure (pic de mémoire résidente, compilation optimisée) : un
  message avec une pièce de 32 Mio montait de **79,3 Mo** avant, **45,9 Mo** après (2,36 puis
  1,37 fois la pièce ; le reste est le message écrit).
- **`f1057649`** (`69706264`, adapté) : une synchro IMAP, JMAP, EWS ou Graph qui n'a fait que
  télécharger d'avance compte ces sources (`SyncReport::fetched`) et signale le dossier changé,
  ce qui relit l'index de recherche. Sonde : `first_sync_takes_a_window`, avec l'étape de
  l'origine, échouait (`touched()` faux après deux sources téléchargées).
- **Vérifié** : chaque sonde avant et après, les suites de `quatrelle-mime`, `quatrelle-engine`,
  `quatrelle-net`, `quatrelle-jmap`, `quatrelle-ui-egui` (le tour et les tests i18n compris) et
  `quatrelle-cli`, `cargo xtask ci` (une réexportation pour les seuls tests, que clippy
  signalait, passée sous `#[cfg(test)]` : `2c446ce1`).
- **Remarqué, non traité** : le téléchargement d'avance en arrière-plan d'EWS (`worker/ahead.rs`,
  le mode Exchange mis en cache) n'émet aucun `FolderChanged`, à la différence d'IMAP, JMAP et
  Graph : les mots des sources qu'il apporte n'entrent dans l'index qu'à sa prochaine relecture du
  dossier. À sonder à part.
- **Reste à faire** : la suite du relevé, R6 (où la règle complète du trombone est à signaler au
  propriétaire).

### État de la récupération : v0.32, quatorzième partie : une recherche qui ne fige jamais la fenêtre (2026-11-05)

Le lot R6 de `merge-tools/survey-v0.32.md` (constat n° 6), sur `main`. La recherche garde son
index en mémoire (l'index sur disque de l'origine, `591adccf` et suite, attend les mesures de
R14). Pour chaque point, sa sonde d'abord, lancée contre le code d'alors.

- **Le texte d'un message sans décoder ses pièces jointes** (`6dee1a15`, adapté de `0a86ddb4`, des
  parties `mime` de `eb1fe5c6` et `248bcbd6`) : `search_text` analysait tout le message qu'il
  indexait, chaque pièce jointe décodée, sur le fil de la fenêtre, à chaque tranche et à chaque
  corps téléchargé. `index_doc` (`mime/src/attachments/index_text.rs`) parcourt la structure MIME
  lui-même (frontières trouvées par `memchr`), ne donne à mail-parser que les en-têtes et les
  parties texte, et borne la profondeur (8), les parties (1 000) et le texte (un million de
  caractères, au lieu de 50 000) ; `search_text` devient `index_doc(raw).joined()`. Un parcours
  coupé le dit (`parts_cut`) : son compte de fichiers n'est pas « aucun ». La recherche lit
  maintenant toutes les parties texte et les messages joints (objet et texte), et ne nomme que
  les vrais fichiers. Cible de fuzz `index_text`, avec le dictionnaire et la graine de l'origine.
  Sondes : un message avec une pièce de 30 Mo coûtait **62 ms** (décodée entière) ; **2,3 ms**
  après. 2 000 fichiers joints donnaient 2 000 noms (moins de 1 000 après, coupé et dit). Les
  tests de l'origine (`mime/src/tests/index_text.rs`, `search_reads_what_a_winmail_dat_holds`)
  ne compilaient pas avant ; le test de l'origine pour `draft_carries_files` n'est pas pris
  (`fefd506a`, lot P2).
- **Le trombone, une règle avec la recherche** (même commit), qui reprend la règle étroite du
  lot R5 (`8d07a07a`). Une partie est un fichier sauf la partie S/MIME, et sauf une partie en
  ligne ou sans disposition que le HTML montre à sa place, quel que soit son nom. **Ce qui
  change dans la liste** :
  - une image que le HTML montre par son adresse (`Content-Location`, RFC 2557, exacte ou par
    son dernier segment) n'a plus de trombone ; sonde : elle en avait un ;
  - la référence doit être entière, en majuscules ou non : `cid:image10` ne montre pas
    `image1`, qui est donc un fichier ; sonde : pas de trombone avant ;
  - **une image en ligne sans nom que le HTML ne montre pas a désormais un trombone** ; sonde :
    elle n'en avait pas ;
  - une partie en ligne qui n'est pas une image (un PDF à `Content-ID`) et que le HTML montre
    par `cid:` n'est plus un fichier ; un PDF en ligne sans `Content-ID` (Apple Mail) en reste
    un ;
  - le `winmail.dat` garde la règle du fork (deuxième partie) : ses fichiers sauf les images
    qu'Outlook cache avec un `Content-ID`, aucun pour celui qui ne tient que le corps, lui-même
    quand le budget en a laissé, sans lire le corps ; la recherche compte de même
    (`tnef_files`). La règle de l'origine (images montrées par le corps du `winmail.dat`) n'est
    pas prise : elle lit le corps pour le trombone.
  - les références du HTML sont lues en une passe, chacune sur 4 Ko au plus. La liste des pièces
    du volet de lecture ne change pas.
- **Un budget en temps** (`c27cd47d`, idées de `5c201bf8`, `ea060907`, `5a5da66b`, sans leur
  moteur) : la fenêtre donnait 500 messages par image, un compte et non un temps. Elle donne à la
  session son horloge (`Session::set_search_clock`, `web-time` ; le moteur n'en lit aucune) et
  6 ms par image : l'index ne commence un message, et la réponse n'en regarde un, que si le plus
  long vu dans l'appel tient encore (trois quarts pour l'index, le reste pour la réponse, qui
  regarde au moins un message par appel). Le compte reste un plafond (4 096 par image) ; une
  recherche demandée entière (ligne de commande, `--cmd`, dossiers de recherche) reste entière.
  Sonde (`tests/searching/paced.rs`) : 3 000 messages, un sur deux téléchargé avec un corps de
  16 Ko, par tranches de 500 : **482 ms** par appel ; après (1 200 messages dans la même
  proportion, le test gardé), l'appel le plus lent qui indexe prend **6,3 ms** sur 239 appels,
  et les 12 attendus sont trouvés.
- **Les requêtes sans mot du texte** (`71a01caa`, adapté de `c0742486` et `df653c4c`) :
  `is:unread`, un drapeau, une taille, une catégorie, « en attente de réponse », ou des mots de
  l'expéditeur, des destinataires ou de l'objet bâtissaient l'index des mots, chaque message
  téléchargé relu. Elles vont à un second index, des résumés seuls (`summary_index`) : aucun
  message lu, rien gardé sur disque, mêmes tranches et même réponse ; l'index des mots reste
  pour la recherche de mots suivante. `Store::reads()` compte les sources lues (`load_body`
  aussi). La partie « index par dossier » de `c0742486` était déjà chez nous. Sonde :
  `summary_searches_read_no_message` (le test de l'origine) échouait, **12 sources lues**.
- **Le dossier affiché d'abord** (`5b7bfd45`, idée de `402ceff4`, qui ordonne chez l'origine les
  candidats de son index sur disque) : les dossiers affichés passent en tête du parcours, les
  autres dans l'ordre du compte. Sonde : `the_folder_on_view_is_read_first`, le dernier de douze
  dossiers affiché, les premières trouvailles étaient toutes dans INBOX.
- **Le téléchargement d'avance d'Exchange** (`23a0f543`, propre au fork) : `worker/ahead.rs`
  signale maintenant le dossier une fois son courrier téléchargé, comme IMAP, JMAP et Graph (une
  fois par dossier, pas à chaque pas : la fenêtre relit alors la liste du dossier). Sonde :
  `downloading_ahead_tells_of_the_folder` (faux serveur EWS, mode Exchange mis en cache, 60
  messages) : rien signalé en 20 s après la synchro complète ; 1,6 s après. **Surprise** : ce
  signal seul n'apportait aucun mot, pour aucun protocole (ni celui de `69706264`) : un dossier
  signalé ne recevait que ses messages nouveaux, pas les sources venues pour des messages déjà
  indexés (un téléchargement d'avance n'envoie pas d'`Event::Body`). Un dossier signalé liste
  maintenant une fois ses sources en cache s'il a des messages indexés sans la leur, et leurs
  mots sont lus au pas de la recherche (`SearchIndex::came`). Sonde :
  `sources_downloaded_ahead_are_searched_once_told`, le mot d'une source arrivée ainsi n'était
  pas trouvé dans la même session.
- **Vérifié** : chaque sonde avant et après, les suites de `quatrelle-mime`, `quatrelle-store`,
  `quatrelle-engine`, `quatrelle-net` et `quatrelle-ui-egui`, `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - l'appel qui finit un dossier écrit son index sur disque et le joint à celui du compte, d'un
    seul tenant : 45 ms pour un dossier de 1 200 messages dont 600 corps (compilation de test),
    davantage pour un gros dossier, une fois par dossier et par session ;
  - un dossier dont l'index est relu depuis le disque lit d'un coup les corps arrivés depuis ;
  - un dossier signalé pendant une recherche liste ses sources en cache (une fois par signal) ;
  - les mesures de R14 sur une boîte de 50 000 messages, avant de choisir l'index sur disque.

### État de la récupération : v0.32, quinzième partie : les périodicités du calendrier, et de petits gels en moins (2026-11-05)

Les lots R7 et R8 de `merge-tools/survey-v0.32.md`, sur `main`. Le calendrier reste celui du
fork : seules les corrections du modèle (`crates/model/src/ical*`) dont le défaut est ici sont
prises. Pour chaque point, sa sonde d'abord, lancée contre le code d'alors.

**R7 — périodicités du calendrier** (invitations .ics et CalDAV ; les séries d'Exchange viennent
de son XML, `ews_pim/events.rs`, et n'ont ni RDATE ni règle horaire : rien n'y change).

- **Les règles d'heure en heure, de minute en minute, de seconde en seconde** (`696dee64`, la
  partie règle de `a5bbc00a`, sans `zone_rule`/`ZoneChange`, qui sert l'export .pst de
  l'origine) : `FREQ=HOURLY`, `MINUTELY` et `SECONDLY` ne donnaient aucune occurrence. Elles
  avancent depuis le début, sous les plafonds habituels, les jours que `BYDAY`, `BYMONTHDAY` et
  `BYMONTH` permettent, à l'heure murale de leur fuseau. Sonde : `rules_within_a_day`, `[]` au
  lieu de `[0, 2, 4, 6]` pour `FREQ=HOURLY;INTERVAL=2;COUNT=4`.
- **`BYHOUR`, `BYMINUTE`, `BYSECOND` et les `RDATE`** (`aabcb994`, adapté de `bb554931`) : ces
  trois parties étaient ignorées, et un `RDATE` finissait dans les lignes inconnues de
  l'événement sans jamais donner d'occurrence. Ils sont lus, servent, repartent au serveur et
  tombent avec la règle (`event_from`, « ne se répète pas »). En plus de l'origine : une série
  déplacée entière emmène ses dates ajoutées, et une occurrence qu'on en détache n'en porte
  aucune. Sondes : `rules_at_hours_minutes_and_seconds`, `BYHOUR=9,10,11` donnait une
  occurrence par heure de 9 h à 14 h ; trois `RDATE` donnaient une seule occurrence, leurs deux
  lignes restant inconnues (le test de l'origine, `added_dates`, a remplacé la sonde).
- **Les lignes gardées repartent entières** (`7a9e5eab`, `whole()` et le CR de `48f8a4a9` ; la
  partie VTODO ne concerne pas le fork, qui ne lit pas les tâches d'un fichier) : un `END`
  égaré dans un événement (`END:VCALENDAR` fermant un composant inconnu) repartait au serveur au
  milieu de l'événement ; une définition de fuseau coupée à la fin du fichier repartait sans
  `END:VTIMEZONE` ; un événement logé dans une définition de fuseau était écrit une seconde fois.
  `whole()` (`ical/whole.rs`) ferme ce qui est ouvert sous son propre nom, laisse un `END` qui ne
  ferme rien et ôte événements, tâches et calendriers d'une définition de fuseau. Un CR au milieu
  d'une ligne est lu comme l'écriture l'écrit, ôté (le CR d'un objet revenait en saut de ligne).
  Sonde : `a_stray_end_is_not_sent_back`, chacun de ses quatre cas échouait (la zone coupée
  avant sa partie `STANDARD` n'était pas écrite du tout ; coupée après, elle sortait
  `(1, 0)`).

**R8 — petits gels et gains sûrs.**

- **`ui.json` sur un fil à lui** (`c0bfe080`, adapté de `2ea4a0be`) : les réglages s'écrivaient,
  avec `sync_all`, sur le fil de la fenêtre, toutes les 5 s où ils changeaient. Un fil
  `settings` écrit les derniers remis, la fermeture attend le dernier ; sans fil, comme avant.
  Sonde : `a_held_writer_thread_returns_hand_overs_at_once`, lancée avec l'écrivain gardé sur le
  fil appelant (la manière d'avant), 100 remises attendaient le disque tenu (2,3 s).
- **Le journal sur un fil à lui** (`d189d040`, de `05e0f2cc`) : chaque ligne s'écrivait sur le
  fil qui journalisait, fenêtre comprise. Un fil `log` les écrit dans l'ordre ; au-delà de 1 Mio
  en attente, elles sont comptées et laissées. Rien n'est perdu à la fermeture : le journal est
  vidé (2 s au plus) avant la sortie macOS d'`on_exit`, quand la fenêtre s'est fermée, et avant
  la sortie « la fenêtre n'a pas pu s'ouvrir ». `8941e8e2` (la sortie de l'écran de démarrage de
  l'origine sur macOS) est sans objet ici. Sonde : `a_line_logged_while_writes_are_held_returns_at_once`
  (par `log::Log::log`), plus de 500 ms d'attente derrière le fichier tenu.
- **S/MIME d'après l'en-tête seul** (`03fda572`, de `1c1ba9ea`) : savoir si un message est
  signé ou chiffré l'analysait entier, à chaque ouverture. `is_protected` ne lit que l'en-tête ;
  `protection()` n'analyse le reste que pour `multipart/signed` ou `pkcs7-mime` (et un `.p7m`
  envoyé en `octet-stream`). Le fork n'a pas de cible de fuzz smime : la vérification
  « jamais dit non protégé à tort » va dans `mime_parse`, avec la graine de l'origine. Sonde :
  `protection()` sur un message non protégé de 30 Mo, **63 ms** (compilation de test) ; le test
  de l'origine, 50 Mo, tient sous 50 ms après.
- **Le message lu, analysé une fois et partagé** (`0ca1a559`, de `13cf8304`) : le volet
  (`s.reading.clone()`) et les fenêtres (`session.windows.clone()`) recopiaient le message entier
  à chaque image, chacun l'ayant analysé ; `unwrap` copiait la source même non protégée. La
  source non protégée est empruntée, l'analyse partagée derrière un `Arc`, une réponse prend sa
  copie. Sonde : `tests/mail/shown.rs::a_message_shown_twice_is_parsed_once` (le test de
  l'origine, plus une mesure) : un message de 5 Mo dans le volet et une fenêtre, **55 ms** de
  copies par seconde d'images.
- **La carte d'invitation sans relire le message** (`dfebab9a`, adapté de `40b3259b`) : elle
  relisait et réanalysait chaque message ouvert pour y chercher un calendrier. Elle regarde
  l'analyse affichée et ne relit la source que s'il y en a un ; si le numéro de la partie ne
  donne rien (un message aux en-têtes protégés se défait autrement pour l'affichage), la source
  est analysée comme avant. Sonde : `a_message_without_an_invitation_is_not_read_again_for_one`
  (`Store::reads()` à la place du compte d'analyses de l'origine), le message ordinaire relu.
- **Les vignettes, les plus anciennes d'abord** (`a5d651de`, version courte de `b580c87a`) : le
  cache des aperçus (images, pages PDF, vidéos) était vidé en entier à la 65ᵉ ; un message de plus
  de 64 fichiers relisait sa source et redécodait chaque vignette à chaque image. Le moins
  récemment montré part le premier, jamais un montré dans l'image en cours, et une puce au-delà
  montre l'icône de son type. Le reste de `321bef22` attend le lot R11. Sonde :
  `tests/reading/previews.rs`, 70 images : **70 textures** refaites (et 70 sources relues) par
  image.
- **Une pièce jointe ne s'ouvre qu'au double-clic** (`c58e2721`, adapté de `a06231b6`) : le
  premier clic d'un double-clic laissait le fichier en aperçu dans le volet, derrière
  l'application qui l'ouvrait. Un clic demande l'aperçu une fois le délai du double-clic passé ;
  un double-clic l'annule. L'allure des puces ne change pas. Les tests des pièces jointes vont
  dans `tests/reading/attachments.rs`, comme chez l'origine. Sondes :
  `a_double_clicked_attachment_only_opens` et son pendant en fenêtre échouaient au premier clic.
- **Ce qui change pour le propriétaire** : les réunions en `RDATE` ou d'heure en heure
  apparaissent ; un clic sur une pièce jointe montre son aperçu après un court délai (le temps
  d'un double-clic) ; un double-clic l'ouvre sans laisser d'aperçu ; plus d'arrêt de la fenêtre
  sur l'écriture de `ui.json` ou du journal, ni sur les gros messages affichés.
- **Vérifié** : chaque sonde avant et après, les suites de `quatrelle-model`, `quatrelle-mime`,
  `quatrelle-engine`, `quatrelle-ui-egui` et de l'application, la cible de fuzz `mime_parse`
  compilée, `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - un événement fait seulement de `RDATE` (sans règle) n'est pas vu comme une série par le
    moteur (déplacer, supprimer, « cette occurrence ») : il se déplace d'un bloc ;
  - le plafond d'entrée de `ical::lines()` saute quand `MAX_INPUT` tombe au milieu d'un
    caractère (`get(..MAX_INPUT)` rend `None`, et tout le texte est lu) ; l'origine l'a corrigé
    dans `bbc8f852` (lot de l'import .pst) ;
  - `apps/quatrelle/src/main.rs` passe 817 lignes (811 avant) : à découper avant d'y ajouter.

### État de la récupération : v0.32, seizième partie : la liste au clavier et la source d'un message (2026-11-05)

Le lot R9 de `merge-tools/survey-v0.32.md` (l'ancien lot 4 de la v0.31, issues #26 et #27), sur
`main`. Le menu à icônes de la liste et le volet de lecture du 04/11 restent tels quels : ce
lot y ajoute, sans rien déplacer. Pour chaque point, sa sonde d'abord, lancée contre le code
d'alors. Les dossiers `engine/src/tests/` et `ui-egui/src/tests/` étant pleins (16 fichiers),
les tests vont dans `tests/mail/`, `tests/list/` et `tests/reading/`.

- **Maj avec les flèches, Début, Fin et les touches de page** (`e1879b18`, de `c18d2dce`) : la
  sélection a une ancre (là où elle commence) et un bout qui bouge. Maj+↓/↑ sélectionnent de
  l'ancre au bout (revenir vers l'ancre en ôte), Maj+Début/Fin jusqu'au premier ou au dernier,
  Maj+Pg préc./suiv. une page (les lignes que la liste montre) ; Début, Fin et les touches de
  page seules vont au message et le lisent. Maj+clic passe par la même commande
  (`list.selectRange`, Ctrl+Maj+clic ajoute la plage), dans l'ordre des lignes montrées. Un seul
  message sélectionné est lu ; plusieurs, le volet dit combien (« 3 messages sélectionnés ») et
  Répondre attend un seul message, comme Outlook. Sondes (`tests/list/selecting.rs`) : Maj+↓
  deux fois laissait un seul message sélectionné ; Maj+clic depuis un message épinglé (montré
  en tête) prenait quatre messages venus d'ailleurs dans la liste, et un second Maj+clic n'en
  ôtait jamais.
- **Les touches de page seules allaient toujours de 10 lignes** (`77611b0d`, propre au fork,
  défaut encore chez l'origine) : `id.ends_with("PageDown")` ne voyait pas `list.pageDown` (p
  minuscule), seulement les formes avec Maj. Trouvé par le test de l'origine lui-même
  (`shift_arrows_select_from_the_keyboard`) : une page de cinq lignes menait à la septième,
  la dernière.
- **Un clic droit sur l'un de plusieurs messages sélectionnés les garde tous**
  (`e7557962`, de `72fc66e0`) : le clic droit ouvrait le message, qui restait seul
  sélectionné ; Supprimer, Déplacer ou Marquer comme lu du menu n'agissaient que sur lui. Sonde :
  `shift_click_and_a_right_click_on_the_selection`, un seul des trois restait sélectionné.
- **Ctrl+A comme Maj** (`650e5419`, de `cb7514bb`) : tout sélectionné, le message lu restait dans
  le volet et Répondre lui répondait. Sonde : `select_all_is_several_selected`, le volet le
  montrait encore.
- **Les touches de la liste seulement dans Courrier** (`168cd816`) : DÉJÀ chez nous
  (`acts_on_mail`, `87113c8c`) ; sa sonde passait telle quelle. Son test reste (`7f0d8ba5`) : il
  couvre Début, Fin et les touches de page dans le Calendrier, les Contacts et les Notes.
- **Pas de Répondre ni d'Impression rapide pour plusieurs messages** (`c6a2c2b8`, adapté de
  `1197321e`) : Répondre, Répondre à tous et Transférer échouaient, Impression rapide demandait
  où enregistrer un PDF puis disait qu'aucun message n'était ouvert. Les quatre sont grisés, avec
  leur icône, sur le menu à icônes du fork ; Supprimer, Déplacer et le reste agissent sur tous.
  Sonde : `the_menu_of_several_has_no_reply`, Impression rapide restait active.
- **Afficher la source du message** (`67688b8b`, adapté de `07ba74e7`) : le fork avait les
  en-têtes seuls (`message.headers`, Fichier › Infos et ⋯ › En-têtes du message, inchangés), pas
  la source entière. Sonde : `message.source` inconnue. La source, telle qu'elle est arrivée,
  s'ouvre dans une fenêtre à elle depuis ⋯ › **Afficher** › **Afficher la source du message**
  (là où Outlook la met), et le même sous-menu Afficher au bas du clic droit de la liste (l'œil
  est l'icône de Cci). Pas de bouton au ruban : l'onglet Affichage d'Outlook n'en a pas.
  « View » d'un menu se dit « Afficher » en français (contexte `menu`), pas « Affichage »
  comme l'onglet. La fenêtre a la barre des fenêtres de message du fork (couleur `chrome`, trait
  dessous) : « Message entier | En-tête seul » en choix `segmented`, « Tout copier » avec son
  icône, la taille ; dessous les lignes en chasse fixe, sélectionnables, seules celles en vue
  mises en page (une source de 30 Mo s'ouvre aussitôt ; les premiers 2 Mo seulement sont
  montrés, la fenêtre le dit, l'enregistrement .eml garde tout). Ce qui pourrait cacher ou
  retourner du texte est montré par son nom (`<U+202E>`), les caractères de contrôle par leur
  image (␀). Un message pas encore là est demandé comme le volet le ferait (`fetch_job` : le
  courrier plus ancien lu sur le serveur, un gros message Exchange son texte d'abord), et la
  fenêtre attend le message entier : chez nous le texte d'abord arrive par son propre événement
  et n'est jamais gardé comme le message, il n'y a donc rien à démêler. `quatrelle-cli source
  KEY [--headers]` l'imprime. Tests en plus de ceux de l'origine :
  `the_source_from_more_actions_view_in_the_forks_window` (le chemin ⋯ › Afficher, la barre
  d'un seul tenant au-dessus des lignes, En-tête seul ôte le corps) et
  `the_source_from_the_lists_menu` ; la tournée sans écran (`tests.rs::tour`) dessine la fenêtre
  entière et en-tête seul, en clair, en sombre et en pseudo-langue.
- **La marque de lettre arabe et les séparateurs de ligne nommés** (`19dea193`, de `fec44fc1`) :
  U+061C retourne le texte comme les autres marques, U+2028 et U+2029 coupaient une ligne où la
  source n'en a pas. Sonde : `hostile_sources`, les trois passaient tels quels.
- **Les lignes jusqu'au pied de la fenêtre** (`0e3ffce5`, adapté de `39587ef5`, sans sa barre de
  droite à gauche) : les lignes en vue étaient comptées avec l'espacement sous chacune, que les
  lignes n'ont pas ; le bas de la fenêtre restait vide. Sonde : `a_source_fills_its_window`,
  36 lignes dessinées dans une fenêtre qui en tient 41.
- **Ce qui change pour le propriétaire** : Maj+↓/↑, Maj+Début/Fin, Maj+Pg préc./suiv.
  sélectionnent au clavier ; Début, Fin et Pg préc./suiv. seuls parcourent la liste ; avec
  plusieurs messages sélectionnés (Maj, Ctrl, Ctrl+A) le volet de lecture se vide et dit combien
  il y en a ; un clic droit dans la sélection agit sur toute la sélection, Répondre et Impression
  rapide y sont grisés ; la source d'un message : ⋯ › Afficher › Afficher la source du message,
  ou clic droit › Afficher.
- **Vérifié** : chaque sonde avant et après, les suites de `quatrelle-engine` et de
  `quatrelle-ui-egui`, `quatrelle-cli` compilé, `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - l'origine a toujours le défaut des touches de page seules (`app/keys.rs`) ;
  - l'ancre n'est remise à zéro qu'à l'ouverture d'un dossier et quand son message quitte la
    liste, comme chez l'origine ; ailleurs (recherche, Boîte d'envoi) le bout l'est seul, et
    une ancre restée dans la liste sert encore au Maj suivant ;
  - `ui-egui/src/lib.rs` est à 983 lignes : à découper avant d'y ajouter.

### État de la récupération : v0.32, dix-septième partie : la flèche Bas et les marques de lu à l'échelle d'une grosse boîte (2026-11-05)

Le lot R10 de `merge-tools/survey-v0.32.md` (constat n° 6, les gels à chaque Bas), sur `main`.
Pour chaque point, sa sonde d'abord, lancée contre le code d'alors ; quand le test de
l'origine ne pouvait pas compiler avant la correction (il appelle ce qu'elle ajoute), la sonde
est une mesure, ou la correction coupée le temps d'un essai. Aucune copie des données du
propriétaire : un banc ignoré, `engine/src/bench.rs::down_twenty` (`f8a71a39`), fabrique une
Boîte de réception de 50 000 messages sur un compte serveur, leurs sources gardées, et mesure
en `--release` ce que coûtent à la fenêtre un Bas et l'écho de sa marque, et à l'écrivain
(son propre fil) l'enregistrement de la marque, en ligne et hors ligne. `session.rs` (846
lignes) a d'abord perdu les types des comptes, pur déplacement (`77dd79fe`).

| Banc, 50 000 messages | avant le lot | après |
|---|---|---|
| un Bas, par date | 17,9 ms | 2,9 ms |
| un Bas, conversations par expéditeur | 114,5 ms | 2,7 ms |
| l'écho de sa marque (`FolderChanged`) | 99 à 188 ms | 1,6 ms |
| un corps téléchargé en route | 196 à 201 ms (dossier relu) | 0,01 ms (`Filled`) |
| l'écrivain, une marque | 192 ms | 18 ms |
| hors ligne, 10 premières → 10 dernières marques sur 100 | 198 → 321 ms | 17,5 → 16,4 ms |

- **Bas et Haut avancent de *n* d'un coup** (`41a34544`, de `e5c03883`) : `list.next` et les
  autres prennent `{"by": n}`, seul le message d'arrivée est ouvert et marqué lu ; ceux passés
  gardent leurs drapeaux. Sondes : `several_steps_read_only_the_message_landed_on`,
  `several_steps_from_nothing_focused` (un pas de 3 n'avançait que d'une ligne). Un compteur
  `messages_shown` sert aux tests.
- **Pg suiv. sans focus revient en haut** (`27d1af5d`, de `53691768`) : le pas de *n* avait
  aussi déplacé Pg suiv. (`Toward::Steps` à part de `Toward::By`). Sonde : Pg suiv. sans focus
  ouvrait la 3ᵉ ligne. La correction des touches de page seules du fork (`77611b0d`) reste.
- **Les appuis groupés en un pas** (`091e1981`, de `0317ccbd`) : les Bas reçus dans une même
  image font une seule commande. Sondes (tests de fenêtre) : 4 appuis lisaient 4 messages, 20
  en lisaient 20 ; maintenant 1.
- **Un index clé → position** (`e67bc133`, adapté de `de32c465`) : `summary`, `index_of`,
  `selected_iter`, `first_selected`, la conversation du message lu sont des recherches et non
  des parcours (`session/mailboxes/nav.rs`) ; un classement par nom met chaque nom en
  minuscules une fois. Sur la liste du fork (des `Arc` partagés avec le cache), `selected()`
  rend les résumés partagés. `proptest` devient une dépendance de test du moteur. Sonde : le
  banc, 114,5 → 45,9 ms par Bas avec conversations par expéditeur (par date 17,9 → 22,9 ms :
  l'index était refait à chaque reconstruction, jusqu'au point suivant).
- **La sélection n'est plus recopiée à chaque image** (`3edcac92`, de `116a89df`) : ruban, menus,
  Supprimer lisent la sélection sur place. Sonde : 20 000 sélectionnés, sept images dessinées
  faisaient sortir 620 000 résumés de la liste ; maintenant aucun.
- **Les lignes gardées entre deux pas** (`ccb7373e`, adapté de `4df14244`) : les lignes (titres,
  conversations, épinglés) sont faites avec la liste (`session/mailboxes/rows.rs`, où va le
  `rows()` du fork) et refaites par `tick()` au changement de jour ; un pas cherche la suivante
  par l'index ; la fenêtre ne recalcule leurs hauteurs que si `nav_structure()` change (avec
  les hauteurs propres au tableau du fork). Sonde (essai provisoire, 20 000 messages lus) : 500
  Bas prenaient 1,1 à 2,1 s selon le classement ; 0,7 à 17 ms ensuite.
- **La marque de lu appliquée en place** (`3ba68ad8`, adapté de `e69ddc54`) : un drapeau ou une
  catégorie change la liste telle qu'elle est quand le résultat est le même
  (`session/mailboxes/patch.rs`), un proptest (`patched_list_equals_refresh`) le tient égal à
  `refresh_list` ; recherche, Boîte d'envoi, supprimés, courrier indésirable et ce que le
  filtre fait entrer reconstruisent. Chez nous l'entrée prend l'`Arc` du cache, et le message
  gardé sous le filtre est `filter_keep`. Sonde (le patch coupé le temps d'un essai) : 100
  messages ouverts reconstruisaient 100 fois les lignes. Banc : 22,9 → 3,3 ms par Bas.
- **L'écho d'une marque ne relit plus le dossier** (`6e4de44e` adapté de `f3753775`, `59fe1f1c`
  de `4cb2576f`) : la session compte, par dossier, les changements envoyés au serveur
  (`session/mailboxes/echo.rs`) et prend un `FolderChanged` qui en attend un pour son écho ;
  un de plus que d'envoyés, un avis du compte ou une minute d'attente relisent le dossier. Les
  tests sont sur un compte Exchange, comme celui du propriétaire. Sondes : 50 ouverts, marqués
  et leur écho reçu sur 10 941 messages reconstruisaient 50 fois la liste (1,93 s, l'écho
  coupé) ; `every_waiting_change_gets_into_a_folder_read_before_them` perdait le premier
  changement ; `a_later_waiting_change_says_how_it_ends` passait déjà (garde-fou). Banc :
  l'écho 113 → 1,6 ms. **Vu en route** (`a3602ca7`) : huit tests du fork (invitations,
  vérification de l'expéditeur) passent un compte en Exchange, ouvrent un message puis livrent
  du courrier par un `FolderChanged` ; sans travailleur, ce changement était pris pour l'écho
  attendu et le courrier n'était jamais lu. Leurs aides prennent les échos comme arrivés ;
  dans l'application, l'écho arrive juste après la marque et relit alors le dossier.
- **Un corps téléchargé remplit sa ligne** (`5f3a23f9`, adapté de `4eb0d977`) : IMAP, JMAP,
  Graph et EWS passent par `keep_source` (`quatrelle-jmap`) et disent `Event::Filled` au lieu
  de `FolderChanged` ; la session met le résumé dans le dossier et dans sa ligne, et ne
  reconstruit que si ce qui a changé décide de ce que la liste montre (conversation, filtre ou
  classement par pièces jointes, taille, importance, Prioritaire). Chez nous un seul chemin de
  téléchargement, les index de recherche prévenus, une réponse remplie dans Brouillons notée ;
  l'application web aussi. Sonde : le banc, 196 à 201 ms par corps téléchargé (dossier relu).
- **Une marque n'écrit que la marque** (`b85338d3`, `7ab836b6` adapté à `Store::apply_op`) :
  dans un dossier en pages, une marque (sur le fil de l'écrivain) lisait toutes les pages, les
  remettait toutes en JSON pour comparer, écrivait la page de 4 096 du message et l'en-tête,
  et rejouait sur une copie de toute la liste chaque modification en attente. Les marques vont
  dans un petit `state-marks.json` à côté des pages (512 au plus, repliées au prochain
  enregistrement complet, vidées par une liste réécrite d'un bloc) ; l'en-tête garde le
  premier numéro de chaque page, si bien qu'une marque ne lit que la page de son message (un
  en-tête d'avant lit tout une fois, le prochain enregistrement le numérote) ; la liste qu'on
  vient de lire a déjà les modifications en attente, elles ne sont plus rejouées. Le fichier
  des marques est lu et écrit sous le verrou du cache, comme une synchro qui le replie.
  `folder_version` ne hache que l'en-tête : une marque le laisse tel quel, et le code le dit
  (ce qu'on en tire, ce qui reste à télécharger, ne dépend pas des drapeaux). Les marques d'un
  compte local enregistrées par la fenêtre (`save_folder_changed`) font de même. Sonde
  (`one_mark_in_a_big_folder_writes_under_64_kib_and_survives_a_reopen`, 10 941 messages) :
  une marque écrivait 1 482 912 octets ; maintenant la marque seule. Banc : l'écrivain
  192 → 100 ms avec le fichier des marques, 18 ms en ne lisant qu'une page.
- **Le compte d'un dossier local après une marque** (`6cd3746b`, propre au fork) : trouvé par le
  test moteur de `7ab836b6` porté ici. `recount_local` comparait les comptes à la liste en
  mémoire, déjà décomptée par `apply_op_memory` : `folders.json` gardait l'ancien nombre de
  non-lus (10 941 pour 10 940), qu'un fichier de données rouvert montrait. Il compare aussi
  avec le disque.
- **Lister un dossier sans parcourir son courrier** (`92aea071` de `cfce0215`, `3bac3ea1` de
  `5953beb5`) : le cache paqueté lit les seules clés et saute chaque sous-dossier d'un pas ; le
  cache en mémoire part de l'endroit où le dossier commence. Sans le banc de l'origine sur une
  copie de vraies données. Sondes (essais provisoires) : les fichiers d'un dossier de 20 000
  sources listés en 2,5 ms → 0,06 ms ; en mémoire, un dossier de 10 messages à côté de 50 000
  en 4,7 ms → 4 à 20 µs. Les tests de `5953beb5` passaient déjà (garde-fous de ce qui est
  listé).
- **Pas pris** : `d3980fdb`, `ef46a988`, `3957819c`, `05d3944d` (les marques des comptes
  locaux et des fichiers de données écrites hors du fil de la fenêtre, machinerie de
  l'origine), ni le `save_marks` de `7ab836b6` (marque et comptes d'un compte local dans une
  seule écriture) : chez nous un compte local écrit encore la marque puis `folders.json`, deux
  écritures sur le fil de la fenêtre.
- **Le coût au carré de `pending_edits`** : il tombe pour l'écrivain (une marque ne rejoue plus
  rien : 100 marques hors ligne, 17,5 → 16,4 ms, sans croissance). Il reste pour les
  enregistrements d'une synchro (chaque modification en attente refaite sur une copie de toute
  la liste, une passe par modification : O(attente × messages) par enregistrement, une fois
  par synchro de dossier au retour en ligne) et pour `ops.json`, relu et réécrit en entier à
  chaque modification ajoutée ou retirée. Correction proposée, non faite : refaire les
  modifications en une seule passe sur la liste (une table numéro → effets, dans l'ordre) ;
  garder le journal par modification (une clé par modification dans le fichier de données)
  plutôt qu'un fichier réécrit.
- **Ce qui change pour le propriétaire** : tenir Bas dans un dossier de 50 000 messages ne
  gèle plus la fenêtre (environ 3 ms par pas au lieu de 18 à 115, puis 100 à 200 ms à l'écho
  de chaque marque et à chaque message téléchargé en route) ; les appuis répétés plus vite que
  l'image ne lisent que le message d'arrivée, ceux passés restent non lus, comme Outlook ;
  l'écrivain suit (18 ms par marque au lieu de 192), y compris hors ligne. Le premier
  enregistrement complet d'un gros dossier (sa prochaine synchro) ajoute les premiers numéros
  de pages à son en-tête ; une version plus ancienne de Quatrelle l'ignore.
- **Vérifié** : chaque sonde avant et après ; les suites de `quatrelle-store`,
  `quatrelle-engine`, `quatrelle-net` et `quatrelle-ui-egui` ; `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - un `FolderChanged` de synchro arrivé entre une marque et son écho est pris pour l'écho : le
    courrier nouveau apparaît à l'arrivée de l'écho (quelques millisecondes). Si cette
    modification échoue ensuite (avis), l'écho ne vient pas et le courrier attend le
    changement suivant du dossier ;
  - l'écho coûte encore 1,6 ms (un passage sur les 50 000 résumés du dossier pour voir s'ils
    ont déjà le changement) ;
  - une sélection de 20 000 (Ctrl+A) cherche encore chaque ligne visible dans la sélection
    (`selection.contains`, `panes/list.rs`, `list/table.rs`) ;
  - un `FolderChanged` d'une synchro relit toujours tout le dossier (200 ms sur 50 000).

### État de la récupération : v0.32, dix-huitième partie : les images du volet de lecture sur des fils de fond (2026-11-05)

La première moitié du lot R11 de `merge-tools/survey-v0.32.md` (R11a), sur `main`. Pour chaque
point, sa sonde d'abord, lancée contre le code d'alors ; quand le test de l'origine ne pouvait
pas compiler avant la correction, la sonde est une mesure écrite ici. Aucune copie des données
du propriétaire : un banc ignoré, `ui-egui/src/tests/pictures/bench.rs`, fabrique un message de
10,4 Mo (36 photos JPEG de 1024 × 768 dans son HTML par `cid:`, et joints trois photos de
2400 × 1800, un PDF de dix pages et une vidéo de 1,5 Mo), l'ouvre dans une fenêtre sans tête de
1280 × 800 en `--release` jusqu'à ce que plus aucune image n'arrive pendant une seconde, puis
prévisualise son PDF ; une fois sans fil (comme le navigateur), une fois avec une voie (un fil
qui fait dans l'ordre le travail confié, comme celui du bureau). `lib.rs` (983 lignes) a d'abord
perdu `UiState`, pur déplacement dans `state.rs` (`2d5398fe`).

| Banc, message de 10,4 Mo | avant le lot | après, dans la fenêtre (navigateur) | après, sur une voie (bureau) |
|---|---|---|---|
| la première image après l'ouverture | 1 751 ms (tout y est fait) | 87 ms | 48 ms |
| la plus lente ensuite | — | 30 ms | 7,4 ms |
| lectures de la source | 42 | 3 | 3 |
| dernière image montrée | 1,75 s | 1,0 s | 0,9 s |
| le PDF prévisualisé : la plus lente, lectures | 40 ms, 11 | 1,1 ms, 0 | 0,9 ms, 0 |

`list.open` lui-même prend 45 ms avant et après (le moteur lit et analyse le message) : c'est
R11b.

- **Les images d'un message extraites en une seule lecture** (`b8addb91`, adapté de
  `321bef22`). Défaut lu dans le code : `picture_texture` décodait dans l'image de la fenêtre et
  relisait toute la source pour chaque image (la fermeture passée à `shown_picture`), comme
  `preview`, `pdf_preview`, `big_picture` et chaque page de `pdf_pages`. (`picture_bytes`, lui,
  ne relisait la source qu'au clic sur « Copier l'image » ; il prend maintenant le fichier déjà
  extrait quand il est gardé.) Un cache par message (`reading/pictures/cache.rs`) extrait les
  fichiers en une analyse, les garde tant que le message est ouvert et décode une image à la
  fois ; les textures sont bornées (96 Mo, les moins récemment montrées partent d'abord, jamais
  une montrée dans l'image), comme les fichiers extraits (128 Mo) ; un PDF ne dessine que ses
  pages à l'écran ; un décodeur qui panique n'arrête que son message. Gardé du fork : les
  images dessinées là où le HTML les place, à la taille qu'il fixe (le cache rend la taille
  propre de chaque image avec sa texture, `Got::Ready(texture, taille)`, et décode une image du
  corps pour l'écran et le zoom, `Want::Body(px)`) ; les vignettes vidéo gardent la LRU de
  `a5d651de` (`previews` devient `video_thumbs`) ; un message dont le texte seul est là ne
  demande rien. Les vignettes sont faites à 160 pixels (montrées à 30 points) au lieu de 480. Le
  cache des images du web et des images portées (une signature) était vidé à la 49ᵉ : il garde
  maintenant l'image où chacune a été montrée (200 au plus, jamais une de l'image en cours).
  Sondes : `a_messages_pictures_come_from_one_read` (douze vignettes, douze images `cid:`, la
  première page d'un PDF) faisait 25 lectures de la source, les 25 décodages dans la même
  image ; maintenant une lecture, une image décodée par image. `many_web_pictures_are_kept_while_shown`
  gardait 30 images sur 250 et en refaisait 750 en trois images ; maintenant 250, aucune. Les
  tests de l'origine viennent ensuite (`tests/pictures/reading.rs`). Deux tests du fork
  suivent : `pictures_past_those_kept_are_not_decoded_each_frame` attend les 70 vignettes cent
  images au lieu de dix (une par image sans fil), `a_picture_waits_for_the_whole_message`
  regarde le cache.
- **Au-delà d'une lecture, un échec oublié, une image trop grosse** (`7e1f2e24`, reprise de
  `ee7ee62f`) : ses trois tests, portés d'abord, échouaient : 256 images montrées sur 300, une
  image en échec jamais réessayée ((2, 2) au lieu de (3, 3) lectures et décodages), les trois
  vignettes lâchées pour une image trop grosse pour être gardée (0 au lieu de 3).
- **Des fils pour le travail lent, un par voie** (`b9d4e38c`, reprise de `1d7292af`) :
  `Services::background(lane, work)` et `has_background`, `Lane::Pictures` et `Lane::Reading`,
  `Work` (le travail, son drapeau d'annulation, « passe d'abord ») ;
  `apps/quatrelle/src/desktop/lanes.rs` fait un fil par voie, le travail dans l'ordre, chacun
  sous `catch_unwind`, et remplace un fil pris 30 s sur un travail annulé. Le `desktop.rs` du
  fork est gardé, les voies y sont ajoutées. Pas de sonde : rien n'existait avant ; ses cinq
  tests passent. La voie de lecture vient entière et sans travail encore : R11b s'en servira.
- **Le décodage sur la voie des images** (`fdb22399`, adapté de `fe861398`) : sonde
  `work_handed_to_the_lane_is_not_done_in_frames`, avec des services qui gardent le travail
  confié : soixante images de la fenêtre lisaient la source une fois et décodaient elles-mêmes
  les 36 images, rien n'allait à la voie ; maintenant (0, 0) dans les images, un travail confié.
  Chaque image faite demande une image de la fenêtre ; une image ouverte en grand ou une page
  de PDF à l'écran passe d'abord. Le navigateur et les tests font toujours le travail une étape
  par image. Tests de l'origine ensuite (`tests/pictures/threads.rs`).
- **L'image d'une vidéo hors du fil de la fenêtre** (`df9a5ce8`, adapté de `51d1cd09`) : sonde
  `a_video_is_written_and_decoded_off_the_window` : deux fichiers étaient écrits dans les images
  de la fenêtre, dont celui d'un message déjà quitté ; maintenant aucun, la vignette vient une
  fois le travail fait, et celui du message quitté ne fait rien (`reading/videos.rs`). La puce
  montre l'icône du type en attendant ; rien n'est demandé sans fichiers (le navigateur) ni
  tant que le texte seul est là. `videos_past_those_kept_make_no_texture_each_frame` (de
  `b580c87a`) passait déjà grâce à `a5d651de` : gardé comme garde-fou. Banc, sur une voie : la
  première image 67 → 48 ms.
- **Pas pris** : `478f363c` (une pièce jointe garde son Content-ID) : aucune sonde ni aucun test
  de ce lot n'en a besoin.
- **Docs** : `docs/architecture.md` dit les deux voies et ce que fait celle des images
  (`17b01e93`).
- **Ce qui change pour le propriétaire** : ouvrir un gros message plein d'images ne fige plus la
  fenêtre (1,75 s sur le banc ; moins de 50 ms maintenant, puis des images de moins de 8 ms) :
  les images arrivent une à une en moins d'une seconde, celle ouverte en grand d'abord ; tenir
  Bas sur de tels messages n'attend plus leurs images (le travail du message quitté est
  annulé) ; un PDF prévisualisé ne relit plus la source à chaque page ; les vignettes des
  pièces jointes prennent un neuvième de la mémoire. Au-delà de 200 pièces jointes images dans
  un message, les puces suivantes montrent l'icône du type (une image ouverte s'affiche
  toujours).
- **Vérifié** : chaque sonde avant et après ; la suite de `quatrelle-ui-egui` et les tests des
  voies ; `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - la première image d'un gros message, sur une voie, prend encore 48 ms, et `list.open` 45 ms
    avant elle : la lecture et l'analyse du message, et son premier dessin, pour R11b ;
  - dans le navigateur, la photo de 2400 × 1800 se décode dans une image (30 ms) ;
  - « Copier l'image » relit la source quand le fichier n'est plus gardé (au clic seulement).

### État de la récupération : v0.32, dix-neuvième partie : le message ouvert lu sur la voie de lecture, et la seule partie visible d'un long message mise en page (2026-11-05)

La seconde moitié du lot R11 de `merge-tools/survey-v0.32.md` (R11b), sur `main` : le lot R11 est
fini. Pour chaque point, sa sonde d'abord, lancée contre le code d'alors ; quand le test de
l'origine ne pouvait pas compiler avant la correction, la sonde est une mesure, ou la correction
coupée le temps d'un essai. Aucune copie des données du propriétaire : des messages fabriqués
dans des tests et trois bancs ignorés en `--release` : celui des images de R11a
(`ui-egui/src/tests/pictures/bench.rs`, message de 10,4 Mo), Bas tenu sur 200 messages d'environ
1 Mo marqués lus au passage (`tests/pictures/bodies.rs::holding_down_frames_stay_under_16_ms`),
et les images d'un long message (`tests/reading/long_text.rs::bench_frames_of_a_long_message`).
`apps/quatrelle/src/main.rs` (817 lignes) a d'abord perdu les commandes `--cmd` du démarrage,
pur déplacement dans `desktop/start.rs` (`16e487ab`). Les deux commits du moteur qui suivent
`0995a69f` (`01e633a1`, `4895be7d`) sont faits avant la fenêtre (`a75755e5`), le moteur d'un
tenant.

| Banc (`--release`) | avant le lot | après |
|---|---|---|
| message de 10,4 Mo, sur une voie : `list.open` | 45,7 ms | 0,0 ms (compte serveur) ; 26,4 ms (compte local : la marque de lu) |
| la première image après l'ouverture, puis la plus lente | 47 ms (celle de la liste comprise), 7,6 ms | 0,6 à 0,9 ms, 1,8 ms |
| Bas tenu, 200 messages d'1 Mo, compte serveur : pire pas (moyenne) | 4,2 ms (3,4) | 0,9 ms (0,6) |
| le même, compte local | 34,5 ms (30,0) | 32,9 ms (27,6) : la marque |
| texte brut de 127 000 caractères : première image, puis chacune | 39,8 ms, 12,1 ms | 4,1 ms, 3,2 ms |
| HTML de 138 000 caractères (2 268 lignes) | 22,7 ms, 7,1 ms | 3,4 ms, 1,1 ms |
| texte brut de 300 000 caractères | 73,7 ms, 30,0 ms | 4,0 ms, 2,8 ms |

Les « 48 ms » de la première image, relevés par R11a, étaient en fait la première image de la
liste qui venait d'être ouverte (46 ms, celle d'avant l'ouverture) : le banc dessine maintenant
deux images avant d'ouvrir le message, et passe aussi sur un compte serveur.

- **Le corps du message ouvert préparé ailleurs** (`43db93d5`, adapté de `0995a69f`) :
  `session/opening/prepare.rs`, `BodyWork`, `finish_body`, `defer_bodies` (coupé par défaut : la
  ligne de commande, `--cmd` et les tests lisent à l'instant) ; un résultat périmé est jeté ; une
  commande autre que se déplacer dans la liste finit d'abord le corps en attente
  (`Session::run`, sous `catch_unwind`). Gardé du fork : un message absent est demandé par
  `fetch_job` (son texte d'abord sur Exchange, ou lu sur le serveur quand il est plus ancien que
  ce qui est gardé) ; le S/MIME est déballé sur le fil de la session ; le texte d'abord arrive
  seul (`Event::TextFirst`) et se montre comme avant, le message entier le remplaçant une fois
  analysé ailleurs ; `show_ready` garde la vérification de l'expéditeur, `keep_senders` et les
  images du web. Sonde (le mode différé coupé le temps d'un essai) : 35 messages montrés, 34
  analysés sur le fil de la session ; maintenant 0. Tests de l'origine portés, avec les sortes
  de messages du fork : invitation, `winmail.dat`, signé, chiffré, signé puis chiffré, et un
  texte d'abord puis le message entier : chacun se lit pareil, différé ou non (même lecture,
  mêmes travaux, même marque, même invitation).
- **Un corps arrivé pendant la lecture du cache** (`77387e6e`, adapté de `01e633a1`) : chaque
  attente porte un numéro, seul le dernier résultat est pris. Sonde : la première moitié du test
  de l'origine demandait un second téléchargement. La seconde moitié (une copie texte d'abord
  dans le cache) n'existe pas chez nous, le texte d'abord n'étant jamais gardé : à sa place, la
  course du fork, le texte d'abord arrivé pendant la lecture du cache ; la lecture, ne trouvant
  rien, redemandait le message. Le texte montré lâche maintenant la lecture attendue
  (`text_shown_first`), le message entier étant en route.
- **Chercher, filtrer, trier sans attendre le corps** (`a0a51894`, adapté de `4895be7d`) : la
  liste des commandes qui laissent le corps à la voie s'allonge (recherche, filtre, Prioritaire,
  tri, volet des dossiers, frappe dans un brouillon). Le test de l'origine passait avant sa
  correction : le compteur des analyses sur le fil de la session ne voyait pas un corps fini là
  par une commande ; il le compte maintenant. Sonde ensuite : un tri inversé lisait et analysait
  le message ouvert sur le fil de la session (1 au lieu de 0).
- **La voie de lecture** (`86739470`, adapté de `a75755e5`) : `reading/bodies.rs` (`app/` a 16
  fichiers), une case à un seul travail : un message passé n'est jamais lu ; la source lue sert
  aux images (`Pictures::give_source`, plus de seconde lecture du cache) ; sans voie (le
  navigateur), le travail se fait dans l'image. Le travail est pris et confié dans `logic()`, qui
  tourne que la fenêtre dessine ou non. Gardé du fork : pendant l'attente, l'objet du message à
  la place de l'en-tête et un petit indicateur discret, pas « Téléchargement du message… ».
  L'application l'active après les `--cmd` et les fichiers passés au démarrage. Sonde (la
  fenêtre n'activant pas le mode différé) : Bas tenu sur 20 messages d'1 Mo, les 20 analysés dans
  une image ; maintenant aucun. `docs/architecture.md` dit ce que fait la voie de lecture.
- **Le banc de Bas tenu** (`31c7f272`, adapté de `0f0a4021`) : 200 messages d'environ 1 Mo,
  **marqués lus au passage** (l'origine coupait la marque, écrite à l'instant chez elle), la
  pire image tenue sous 16 ms sur un compte serveur, celui du propriétaire, où la marque part chez
  l'écrivain ; sur le compte local de l'exemple, la fenêtre écrit la marque (dit, non tenu). La
  sonde passait déjà sur le compte serveur (4,2 ms) : un message d'1 Mo de texte se lit et
  s'analyse en 3 ms environ. C'est le message de 10 Mo du banc des images qui coûtait 20 ms à
  lire et analyser, et 25 ms de plus pour la marque sur un compte local.
- **La seule partie visible d'un long message mise en page** (`7ec8ec4e`, adapté de
  `7020f7bb`) : `reading/long_text.rs` ; le texte brut de plus de 20 000 octets en morceaux de
  4 Ko, le HTML de plus de 2 000 lignes en blocs de 64 lignes, les tableaux de sa mise en page un
  à un ; hauteurs estimées puis mesurées, deux de plus par image. Gardé du fork : les adresses
  d'un long texte brut restent des liens (une main, l'adresse, ouvertes d'un clic, comme
  `plain.rs`) ; un tableau non dessiné garde les numéros de ses cellules (le premier de chaque
  tableau calculé d'après ceux d'avant), si bien que les hauteurs qui alignent ses lignes ne sont
  pas prises par les cellules d'un autre (`CellHeights`) ; le texte suit le zoom du message.
  Sondes : le banc ci-dessus, les seuils coupés le temps d'un essai. Tests de l'origine portés ;
  celui du Maj+clic vise plus bas, l'en-tête du fork restant en place au-dessus du message.
  Tests du fork : les liens d'un long texte brut ; les tableaux d'une longue lettre
  d'information (sans les numéros gardés, sept tableaux se partageaient quatre numéros).
  **`0eeb5d6a`** (la sélection d'un seul tenant, phase 4) n'est pas nécessaire pour que la copie
  soit juste : Ctrl+A puis Copier donne tout le texte, et une sélection au Maj+clic d'un bout à
  l'autre copie exactement le texte entre les deux (testés). Il manque pour le clavier :
  Maj+flèches, Début et Fin s'arrêtent au bord d'un morceau, et un glisser hors du volet ne fait
  pas défiler. Un long texte brut n'est plus une zone de texte : Ctrl+A y demande une sélection
  commencée dans le texte.
- **Deux préparations finies dans le désordre** (`4ee41366`, adapté de `eb563510`) : le travail
  est numéroté, seul un corps plus récent est gardé. Sonde (test du fork : un long HTML ouvert,
  puis le suivant, l'ancien fil analysant encore quand le nouveau a fini) : 3 passes sur 3
  laissaient le volet vide à attendre ; maintenant le message s'affiche.
- **Ce qui change pour le propriétaire** : tenir Bas ne lit et n'analyse plus aucun message sur
  le fil de la fenêtre : la voie de lecture le fait, et seul le message d'arrivée est lu ;
  ouvrir un gros message ne coûte plus rien à la fenêtre (45 ms avant pour 10 Mo), il s'affiche
  une image ou deux plus tard, son objet montré entre-temps ; chercher, filtrer ou trier
  n'attendent plus le message ouvert. Un long message (une longue conversation en texte, une
  longue lettre d'information) ne fige plus la fenêtre à l'ouverture (40 à 74 ms) ni à chaque
  image ensuite (12 à 30 ms) : 3 à 4 ms. Répondre, transférer, enregistrer agissent toujours sur
  le message tel qu'il s'affiche.
- **Vérifié** : chaque sonde avant et après ; les suites de `quatrelle-engine` et de
  `quatrelle-ui-egui` ; `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - sur un compte local ou un fichier de données, la marque de lu s'écrit encore sur le fil de la
    fenêtre (25 ms par message dans les bancs) : le `save_marks` de `7ab836b6`, pas pris en R10 ;
  - restent sur le fil de la session : le message signé ou chiffré (déballé puis analysé), la
    copie texte d'abord d'Exchange (petite), un message ouvert dans sa propre fenêtre, et
    l'indexation pour la recherche d'un corps téléchargé (`index_body`) ;
  - dans le navigateur, sans voie, le travail se fait dans l'image (60 ms pour un HTML de 138 000
    caractères) ;
  - la première image après l'ouverture d'un dossier coûtait 46 ms dans le banc des images : non
    regardé ;
  - `reading.rs` est à 799 lignes : à découper avant d'y ajouter quoi que ce soit ;
  - le commit `43db93d5` laissait un avertissement de clippy, corrigé dans `77387e6e`.

### État de la récupération : v0.32, vingtième partie : un message ouvert pendant une synchro arrive tout de suite, et le texte d'abord pour IMAP (2026-11-05)

Le lot R13 de `merge-tools/survey-v0.32.md` (issue #24 de l'origine, `46ca26a5`), sur `main`, en
deux parties ; pour chacune, sa sonde d'abord, lancée contre le code d'alors. Aucun vrai compte :
les faux serveurs IMAP (`fakes/fake.rs`) et EWS (`fakes/fake_ews`). D'abord deux déplacements purs :
le code d'ouverture quitte `worker/sync.rs` (884 → 784 lignes) pour `worker/opening.rs`, comme chez
l'origine, et les lecteurs de dossiers `worker.rs` (869 → 786) pour `worker/connect.rs`
(`515cd78a`) ; les commandes de dossiers du client IMAP quittent `protocols/imap.rs` (870 → 751)
pour `protocols/imap/folders.rs` (`6515f78c`).

| Sonde | avant | après |
|---|---|---|
| IMAP, ouvert pendant les 8 s de la synchro de l'Archive (test de l'origine) | 8,1 s | 0,42 s |
| EWS, ouvert pendant les 8 s de la lecture d'un dossier : un petit message, le texte d'un gros | 7,8 s, 7,85 s | 0,58 s, 0,48 s |
| EWS, ouvert pendant les 8 s de la lecture des calendriers et des contacts | 8,0 s | 0,50 s |
| EWS, dossier public et boîte partagée pendant une synchro | — | 0,78 s, 0,52 s, chacun par sa route |
| IMAP, message avec un fichier de 20 Mo (27 962 771 octets), ligne de 10 Mo/s : le texte affiché | après 3,19 s et tous ses octets | après 0,42 s et 967 octets ; le message entier à 3,6 s |

Les 400 ms de grâce (ci-dessous) sont comprises dans les chiffres d'après.

- **Ouvert pendant une synchro, sur une copie de la connexion principale** (`d1c17246`, adapté de
  `46ca26a5`). L'origine envoie le message sur la seconde connexion du compte ; le fork l'avait
  refusé le 02/11 (pas de numérotation sur Exchange, pas les routes des dossiers publics et des
  boîtes partagées, et elle sert à l'envoi et aux salles). La règle est gardée : la connexion
  principale dit quand elle travaille (`Worker::at_work` : lecture d'un dossier, liste des
  dossiers, calendriers et contacts, calendriers cochés) et prend alors une copie d'elle-même, le
  `sibling()` du client EWS avec l'id de chaque dossier sur le serveur, ou, en IMAP, le compte et
  ce avec quoi il se connecte. Un message ouvert pendant ce temps va toujours dans la file de la
  connexion principale, et aussi à `Opened` (`worker/opening.rs`) : si la connexion principale ne
  l'a pas commencé 400 ms plus tard, un fil à lui l'apporte sur la copie, le dernier ouvert
  d'abord ; le premier qui le commence l'a (`Opened::claim`). Sur Exchange la copie trouve l'id du
  message par son Message-ID dans son dossier (un `FindItem`, `ews::find_by_message_id`) : la
  numérotation (550 000 ids) reste à la connexion principale et n'est jamais relue pour cela. Un
  gros message vient en texte d'abord (`Event::TextFirst`), puis entier sur le fil des messages
  entiers (`Rest`, du lot 2) ; un petit vient entier ; seule la source est écrite, jamais la liste
  du dossier. Gardé du lot 2 : une copie refusée à la connexion ne coûte qu'un refus pour toute la
  session (`Rest::refuse`, partagé maintenant par les copies), la connexion principale apportant
  les messages après son travail ; le refus du texte seul par le serveur est partagé aussi ;
  S/MIME, invitations et « Tout télécharger » restent entiers ; l'expéditeur lu par `0x5D01`.
  Graph et JMAP attendent la connexion principale comme avant (leur copie aurait besoin de la
  numérotation, et Graph n'a pas ici de recherche par Message-ID). Le faux IMAP apprend le
  `stall_on` de l'origine, le faux EWS des calendriers retenus une fois (`slow_pim`). Tests du
  fork en plus : un refus de copie pendant une synchro (un seul refus, les deux messages viennent
  par la connexion principale, aucune copie essayée ensuite), un dossier public et une boîte
  partagée pendant une synchro.
- **Le texte d'abord pour IMAP par `BODYSTRUCTURE`** (`e90c4de6`, adapté de `46ca26a5`). Le moteur
  demande le texte d'abord d'un message IMAP de 256 Ko ou plus, ni signé ni chiffré
  (`IMAP_TEXT_FIRST_OVER`), fichiers ou non : l'en-tête seul ne le dit pas, la structure le dira.
  Le client lit la structure (`protocols/imap/text_first.rs`) : une seule partie, signé ou chiffré,
  sans fichier, ou au-delà des plafonds (500 parties, 16 niveaux, 32 parties texte, 8 Mo de texte ;
  l'imbrication est déjà bornée à 64 avant l'analyse, R1) : entier. Sinon l'en-tête et les parties
  texte seules, l'en-tête de chacune fait depuis la structure (jetons de 64 caractères au plus,
  sans retour à la ligne) ; les fichiers nommés (RFC 2231 et RFC 2047 décodés, `mime::part_name`)
  et pesés. Le message entier vient sur une connexion de plus, connectée comme la principale
  (`Whole::Imap` sur le fil de `Rest`, gardée d'un message au suivant), pas sur la seconde
  connexion du compte, qui envoie. Le faux IMAP apprend `BODYSTRUCTURE` et les sections de parties
  (`fakes/fake/structure.rs` de l'origine, son découpeur passé à une lecture par ligne : il mettait
  1,4 s à couper 28 Mo en compilation de débogage) et un débit (`rate`). Fuzz : la cible `imap`
  demande aussi un texte d'abord, comme chez l'origine, et `text_first` lit les réponses d'un
  serveur IMAP (premier octet, graine `seeds/text_first/imap`, mots du dictionnaire `imap`).
  `cargo fuzz` n'étant pas installé, les cibles sont seulement compilées (`cargo check` dans
  `fuzz/`). Les tests contre de vrais serveurs (`fakes/real_servers.rs`, GreenMail) ne sont ni
  portés ni lancés.
- **Ce qui change pour le propriétaire** :
  - Exchange : un message ouvert pendant que la connexion principale lit un gros dossier (la
    première synchro, Envoyer/Recevoir, un dossier public ouvert) ou les calendriers et les
    contacts s'affiche en moins d'une seconde au lieu d'attendre la fin de ce travail (des
    minutes à la première synchro) ; un gros, son texte d'abord. Le message entier finit toujours
    dans le cache (mode Exchange mis en cache). Coût : un `FindItem` par message ouvert ainsi, et
    une connexion de plus, qui se connecte en NTLM.
  - Gmail et Yahoo : un message de 256 Ko ou plus (une pièce jointe) montre son texte et la liste
    de ses fichiers tout de suite, le reste suit ; ouvert pendant une synchro, il ne l'attend plus
    non plus. Une connexion de plus le temps d'un téléchargement (deux pendant une synchro).
- **Vérifié** : chaque sonde avant et après ; les suites de `quatrelle-net`, `quatrelle-mime`,
  `quatrelle-model` et `quatrelle-engine` ; les cibles de fuzz compilent ; `cargo xtask ci` (wasm
  compris).
- **Reste, remarqué** :
  - rien n'a été essayé sur le vrai Exchange : le coût du `FindItem` par Message-ID dans un dossier
    de 100 000 éléments n'est pas connu (la première fois, Exchange peut bâtir une vue de
    restriction) ; un message sans Message-ID attend la connexion principale ;
  - un message apporté par la copie ou par `Rest` ne remplit pas son résumé (`Event::Filled`) :
    en IMAP, l'aperçu de la liste reste vide pour lui, comme au lot 2 sur Graph ;
  - Graph et JMAP ne sont pas couverts (voir plus haut) ;
  - un compte IMAP en OAuth dont le jeton a expiré ne prend pas de copie (il attend) ;
  - le texte d'abord IMAP ne porte pas `In-Reply-To` (le constructeur du fork ne l'écrit pas) : le
    fil se fait au message entier ;
  - surpris en route : la première forme (la copie gardant le message pendant les 400 ms) rendait
    le message trop tard, la connexion principale étant déjà repartie, et un test du lot 2 a
    déraillé ; d'où la double file et `claim`. Un test Teams (`check_caps_listings_at_ten_pages`)
    a échoué une fois sous charge et passe seul ;
  - `worker/changes.rs` (827 lignes) n'a pas été touché ; `worker/` a maintenant 15 fichiers.

### Hors relevé : changer la portée de la recherche figeait la fenêtre (2026-11-05)

Signalé par le propriétaire sur la `1.2.0~dev.3` : passer de « Dossier actuel » à « Boîte aux
lettres actuelle » a figé la fenêtre pendant des minutes. Les piles du processus montraient le fil
de la fenêtre dans `search_now` → `index_step` → `read_body` / `search_text` / `Index::add` :
toute la boîte Exchange indexée d'un bloc. Défaut du fork depuis le 22/10 (`6590f6ec`), pas
de la dev.3 : seule la frappe dans la zone demandait la recherche par tranches (`"index"`) ; le
menu de portée (zone, onglet Recherche, en-tête de la liste), une recherche récente, la recherche
avancée et un dossier de recherche ouvert, créé ou modifié la demandaient entière, et R6 n'avait
réglé que la frappe.

- **Corrigé dans le moteur, à un seul endroit** : une recherche demandée sans tranche à une
  session qui a l'horloge de la fenêtre (`set_search_clock`) est une tranche aussi
  (`pace::WINDOW_SLICE`, 4096 messages au plus, la part d'image la tenant à une image) ; la ligne
  de commande et les tests, sans horloge, la font toujours entière. Un bouton ajouté plus tard ne
  peut plus l'oublier.
- **`search.more`** : la fenêtre poursuit la recherche affichée par cette commande, dans sa propre
  portée et comme son propre dossier de recherche ; poursuivie comme une recherche tapée, celle
  d'un dossier de recherche aurait pris la portée de la zone et cessé d'en être un.
- **Sonde** (`ui-egui/src/tests/list/searching.rs`, 1 200 messages dont 600 avec un corps de
  16 Ko) : l'appel du menu prenait 988 ms et finissait tout ; il prend 3 ms, et la recherche finit
  en 308 images avec les mêmes 12 trouvés. Un dossier de recherche créé depuis sa boîte de
  dialogue, la zone étant sur « Dossier actuel », reste lui-même jusqu'au bout et la zone garde sa
  portée.

### État de la récupération : v0.32, vingt et unième partie : « Compacter maintenant » rend la place des pièces jointes gardées brutes (2026-11-05)

Le lot R12 de `merge-tools/survey-v0.32.md` (constats n° 1 et 6), sur `main`, selon la décision du
propriétaire du 05/11 (n° 6) : **rien ne réécrit le cache tout seul**, tout passe par le bouton
« Compacter maintenant ». Pour chaque partie, sa sonde d'abord, lancée contre le code d'alors, sur
des fichiers de données fabriqués (aucune copie du cache du propriétaire). D'abord un déplacement
pur : `packed/tests.rs` (821 lignes) découpé par sujet en `packed/tests/` (`names`, `kept`,
`opening`, `compaction`, `folders`), comme l'origine l'a fait (`e32a7bd2`).

| Sonde | avant | après |
|---|---|---|
| 200 Mo de pièces jointes en base64 gardées brutes (fichier écrit comme avant R3, compacté) | 213 061 632 → 213 061 632 octets, rien de rendu | → 162 304 000 octets (0,762) |
| 1 Go, même fabrication (banc ignoré, `--release`, cache disque chaud) | — | 1 090 785 280 → 830 345 216 octets (0,761) en 4,7 s |
| Une lecture du fil de la fenêtre pendant un compactage (retenu 1,5 s par un autre utilisateur du fichier) | 1,58 s d'attente | refusée en 27 µs |
| Compactage d'un fichier sans rien à rendre (306 Mo gardés dans 539 Mo) | 623 382 528 octets (2,04 fois le contenu) | 311 758 848 octets (1,02) |

- **Défaut du fork trouvé en route : le scellement après compactage doublait le fichier**
  (`2ee90630`, propre au fork). Le commit scellé que R2 avait ajouté après `db.compact()` (pour
  qu'un arrêt juste après ne demande pas la vérification complète) avait besoin de quelques pages
  sur un fichier compacté sans place libre ; redb agrandit alors le fichier en le doublant tant
  qu'il fait moins de 4 Gio (taille de ses régions). Vu d'abord sur la sonde de 200 Mo
  (162 271 232 octets avant le scellement, 324 538 368 après), puis reproduit sans recompression
  à partir d'environ 300 Mo gardés. Le fichier est maintenant fermé puis rouvert : à la fermeture,
  redb enregistre l'état de l'allocateur depuis ses pages les plus basses et rogne la fin (le cas
  de leur issue 1165). Un arrêt brutal après le compactage ne demande toujours aucune réparation
  (`a_data_file_killed_after_compacting_needs_no_repair`).
- **La recompression** (`86802090`, adapté de `ccf5f42f`, `79de1aa5`, `eac8e93a`, `4f130900`) :
  `Store::recompress_data_file` (`packed/recompress.rs`) et `Store::compact_now`, qui fait ce que
  fait le bouton (recompresser, puis compacter en place). Du code de l'origine : lots de 8 Mio au
  plus, lus dans une transaction de lecture, compressés hors du fichier (quatre fils), écrits dans
  une transaction courte, **scellée comme toutes celles du fork**, avec le curseur dans une table
  `META` (`packed/meta.rs`) ; un fichier changé depuis sa lecture est laissé tel qu'écrit, sans
  refaire les empreintes quand rien n'a été écrit entre-temps (`Writes`) ; plus d'écriture dès que
  le compte est supprimé (`Store::while_kept`) ; un déplacement derrière le curseur pose un point de
  retour (`recompress.back`) ; un fichier de plus de 64 Mio est laissé. Adapté : pas de `Commit`
  ni de `Seals` (le fork scelle chaque commit), pas d'index `search/` à sauter (l'index du fork,
  `search.idx` par dossier, est un fichier comme un autre), les anciens préfixes du fork
  (`DEFLATED_OLD`) comptent comme compressés, la progression compte les fichiers (`len()` de la
  table, sans la parcourir) et reprend au compte noté dans `META` (`recompress.looked`). De
  `eac8e93a`, seulement `META` : un fichier créé par cette version le dit aussitôt, dans un commit
  scellé, et n'est jamais parcouru ; **la mesure de santé n'est pas prise**, le bouton n'en a pas
  besoin (la recompression dit elle-même ce qu'elle a gagné). Une recompression qui échoue (une
  `META` d'un autre type, le disque) n'empêche pas le compactage. Débit mesuré : 224 Mo/s sur
  200 Mo, 271 Mo/s sur 1 Go (huit cœurs, cache chaud).
- **La fenêtre n’attend plus le compactage** (`053ab0ad`, l'idée de `0151859a`) : le fil de la
  fenêtre est marqué (`quatrelle_store::no_waiting()`, appelé avant `run_native`) ; ses lectures
  d'un fichier en compactage échouent tout de suite (`WouldBlock`, « being compacted ») ; ses
  écritures, l'écrivain et les travailleurs attendent comme avant. La copie et l'échange de fichiers
  de `0151859a` ne sont pas pris : le fork compacte en place.
- **Le bouton** (`7487b18c`) : le fil `compact` appelle `compact_now`. Phase 1, les pièces jointes
  sont réduites, la fenêtre restant utilisable : Fichier › Informations dit « Réduction des pièces
  jointes gardées ici : 42 % (vous pouvez continuer à travailler) », avec une jauge
  (`Event::CompactProgress`, cinq fois par seconde au plus). Phase 2, le compactage en place, sous
  le voile que la fenêtre avait déjà et qui ne paraît plus que pour elle ; pendant ce temps la
  fenêtre ne lit ni aperçus ni recherche pour ce compte (ses lectures seraient refusées, et les
  messages pris pour absents) ; les deux reprennent ensuite. À la fin, comme avant « Compacté :
  2,3 Go → 1,8 Go », suivi de « dont 0,4 Go par la réduction des pièces jointes ». Pendant la
  phase 1, redb peut doubler la taille du fichier avant que la phase 2 ne la rende : sans cette
  place et 1 Go de marge sur le disque (`fs4::statvfs`), le bouton compacte seulement et le dit.
  Trois textes nouveaux dans les treize catalogues ; « réduire » plutôt que « recompresser » :
  plusieurs langues disent déjà « compresser » pour compacter.
- **La reprise** : les tests de l'origine, portés (avant : l'API n'existait pas, rien ne
  compilait) : arrêtée après un lot puis reprise d'un magasin rouvert (chaque fichier compressé une
  fois, la progression repart du compte noté) ; un processus tué après le 1er, le 2e ou le 4e lot
  (le fichier rouvre **sans réparation**, rien de perdu, la reprise finit) ; un compte supprimé
  entre la lecture et l'écriture d'un lot ou entre deux lots, aussi par `compact_now` (aucun fichier
  recréé ni nommé) ; un message déplacé derrière le curseur pendant l'écriture d'un lot, entre deux
  lots et pendant l'arrêt (compressé quand même) ; des fichiers réécrits pendant un lot (laissés).
- **Le hostile** : `META` aux valeurs trop courtes, curseur qui n'est pas du texte ou plus long
  qu'une clé, point de retour trop long : tout se relit, la recompression repart de zéro, puis le
  compactage. Un curseur après toutes les clés s'arrête proprement (rien de compressé, compacté
  quand même) ; une `META` d'un autre type de table : compacté quand même. Fuzz : la cible
  `data_file` de l'origine (`a4b82e81`), sa graine (faite par le test du magasin) et son
  dictionnaire (préfixes du fork, anciens et nouveaux) ; seulement compilée, `cargo fuzz` n'étant pas
  installé.
- **Les bancs** (`e0ff9427`, l'idée de `047809d3`), ignorés, en `--release` : 200 Mo et 1 Go
  écrits par transactions de 64 Mo et comparés par empreintes (1 Go ne tient jamais en mémoire).
  Sur 1 Go, la taille apparente du fichier monte à 2,18 Go pendant la phase 1, mais le fichier est
  creux : 1,10 Go sur le disque. À côté du bouton (200 Mo), les écritures de la fenêtre prennent
  3,8 ms (médiane), 17,3 ms (p99) pendant la phase 1 ; ses 73 lectures pendant la phase 2 sont
  toutes refusées, en 0,044 ms au plus.
- **L'arrêt pendant le compactage en place** : redb fait tout le compactage dans un seul appel,
  dont les commits sautent l'enregistrement de l'allocateur (son issue 829) ; aucun commit scellé
  ne peut s'y glisser sans changer redb, donc un arrêt net à ce moment demande toujours la
  vérification complète au démarrage suivant. Réduit (`4cf77b7f`) : cette phase ne compacte plus que
  ce que la phase 1 a libéré (0,74 s pour 1 Go ici) ; fermer la fenêtre pendant elle attend sa fin
  (le voile dit « Quatrelle se fermera dès que ce sera fini »), puis ferme avec les questions
  habituelles ; fermée pendant la phase 1, elle ferme tout de suite (lots scellés, reprise au
  prochain clic). La mise en veille ne tue rien. Le fil `compact` rattrape aussi une panique et la
  dit, pour que le voile, le bouton et cette fermeture n'attendent jamais une réponse qui ne vient
  pas. Restent la coupure de courant et l'arrêt forcé pendant ces secondes.
- **Laissés** : l'interface de compactage de l'origine (`6a7ec801`, `1b1f0454`), le compactage au
  repos ou à la fermeture (`6ed4dccc`, `2c09cf1a`, `f7d188a3`), les écritures non durables
  (`652ac8f6`, `70ee4080`, `c769fc09`, `6a83e16d`), la mesure de santé, le compactage par copie.
- **Ce que verra le propriétaire** (cache de 2,3 Go, environ 550 000 éléments, estimations à partir
  des bancs, la part de pièces jointes brutes de son fichier n'étant pas connue) : phase 1 de
  l'ordre de 10 à 30 s, la fenêtre utilisable, la jauge dans Fichier › Informations ; phase 2 de
  quelques secondes sous le voile ; un fichier d'environ 1,8 à 1,9 Go si les trois quarts sont des
  pièces jointes brutes (un quart de celles-ci rendu), plus la place du courrier supprimé. Il faut
  environ 3,3 Go libres sur le disque pour la phase 1, sinon compactage seul. Un second clic ne
  refait pas la phase 1 (`META`).
- **Vérifié** : chaque sonde avant et après ; les suites de `quatrelle-store`, `quatrelle-engine`,
  `quatrelle-net` et `quatrelle-ui-egui` (tests i18n compris) ; la cible de fuzz compile ;
  `cargo xtask ci` (wasm compris : le magasin en mémoire du navigateur n'a pas de fichier de
  données, le bouton y répond comme avant).
- **Reste, remarqué** :
  - après un compactage serré, la première écriture fait doubler la taille *apparente* du fichier
    (croissance de redb, fichier creux sur Linux) ; sur Windows (NTFS), la place est réservée pour
    de bon. **Réglé aussitôt pour l'affichage** : « Conservé sur cet ordinateur » et le bilan
    avant → après du bouton lisent la place prise sur le disque (`store/src/lib.rs::on_disk` : la longueur,
    moins les trous sous Unix), plus la longueur. Sonde
    `an_accounts_size_is_the_room_its_file_takes` : 40,2 Mo gardés, fichier long de 82,3 Mo après
    le premier message écrit, 82,3 Mo affichés ; 41,4 Mo maintenant ;
  - la vérification de place est prudente (taille apparente + 1 Go) ;
  - aucun bouton pour arrêter la phase 1 (la décision n'en demande pas) : fermer Quatrelle l'arrête,
    le clic suivant reprend ;
  - le doublement à la croissance de redb concerne toutes les écritures, pas seulement le bouton ;
    il explique sans doute une part de la taille des fichiers des propriétaires.

### État de la récupération : v0.32, vingt-deuxième partie : la recherche ignore les accents et coupe les mots aux apostrophes ; ce que coûte l'index (2026-11-05)

Le lot R14 de `merge-tools/survey-v0.32.md`, sur `main`, selon la décision du propriétaire du 04/11
(n° 3 et 4) : **accents ignorés et mots coupés aux apostrophes**, dans le courrier et le calendrier ;
**la requête envoyée au serveur reste telle que tapée** ; l'index sur disque (P3) attend ces mesures.
Pour chaque partie, sa sonde d'abord, lancée contre le code d'alors, sur des boîtes fabriquées. D'abord
un déplacement pur : `model/src/search.rs` (764 lignes) découpé par sujet en `search/parse.rs`,
`matching.rs`, `index.rs` (`df1b68d2`), comme l'origine l'a fait dans `41464ec1`.

| Sonde | avant | après |
|---|---|---|
| `equipe` dans « Réunion Équipe », `resume` dans « Le Résumé », `équipe` dans « Notes equipe » (`accents_are_read_away_both_ways`) | rien trouvé | trouvés, index et vérification |
| `PRÓJECTOR` dans la boîte d'exemple (`search_folds_accents`, la partie pliage du test moteur de `41464ec1`) | 0 | 1 |
| `equipe` dans « l'équipe » et « l’équipe », `l'equipe` dans « l’équipe », `hui` dans « aujourd'hui », `word` dans « site.word », `list` dans « shot_list_v2 » (`joined_words_are_found_by_their_parts`) | rien trouvé | trouvés ; « 10,000 » pas coupé, une lettre n'est pas une partie |
| un événement « Réunion d'équipe », salle « Hélène », cherché par `equipe`, `helene`, `"reunion d'equipe"` | introuvable | trouvé |
| `helene` dans les personnes (« Hélène Dupré »), `dupre` dans le carnet, `MÁRTÍN` dans les tâches (« Devis Martin ») | rien | trouvés |
| `"thank you for the update"` dans 50 000 messages, dont des HTML mis en texte (`a_phrase_is_found_across_a_line_break`) | 16 643 sur 16 667 | 16 667 |

- **La table de pliage** (`6b8c1bde`, repris de `82a959b4`) : `cargo xtask fold` l'écrit dans
  `model/src/search/fold_table.rs` avec `icu_normalizer` (NFKD, accents retirés, ß, æ, ø, œ, ł, đ, ð,
  þ, ı, ё, ς à la main), le modèle portable n'emporte aucun normaliseur ; `fold --check` entre dans
  `cargo xtask ci`, après `authors`. Adapté : la table s'écrit avec l'en-tête d'auteurs du fork. La
  table faite ici est celle de l'origine, à l'en-tête près.
- **Le pliage des deux côtés** (`9838da6c`, adapté de `41464ec1`, sa partie pliage seulement) :
  `search/fold.rs` (`fold_word`, `tokens`, du code de l'origine), appliqué où l'index prend les mots,
  où la requête interroge l'index, et où un message est vérifié (mots, mots exclus, phrases pliées
  lettre à lettre, adresses). `Term::Words` garde les mots tapés : EWS, IMAP, Graph et JMAP reçoivent
  la requête telle quelle (`a_server_is_asked_for_the_words_as_typed`, déjà juste, gardé en
  garde-fou). `FOLDER_VERSION` passe à 2 : les `search.idx` anciens sont relus. **Pas pris** : les
  dates (`received:`, `sent:`, `date:`, `after:`, `before:`, `search/dates.rs`), `attachment:`, la
  logique à trois réponses ; le pliage s'en sépare proprement et se pose sur la logique à deux
  réponses du fork. Pris aussi : le test de pliage de l'origine, son proptest d'idempotence, les
  entrées du dictionnaire de fuzz ; deux proptests écrits ici (une requête quelconque vérifiée sans
  panique ; un mot tapé avec ou sans accents trouve le texte qui l'a).
- **Les parties des mots joints** (`1191f0e8`, adapté de `282b08c6`) : `index_tokens`, `parts`,
  `joins` de l'origine ; chaque partie de deux lettres ou plus est indexée et vérifiée. Propre au
  fork : l'apostrophe typographique (’, ‘) se lit comme la droite, dans les mots et les phrases.
  `FOLDER_VERSION` passe à 3. Deux tests du moteur comptent « shot list » deux fois, comme chez
  l'origine (« shot_list_v2 »). `store/src/search/writer.rs` est sans objet ici.
- **Le calendrier** (`c3589feb`, propre au fork : l'origine ne plie pas `ical/find.rs`) : chaque
  champ plié lettre à lettre, ce qui règle aussi le sigma final que la présélection devait excuser ;
  les mots et leurs parties comme le courrier. La référence du test d'équivalence (3 000 cas) lit de
  même, apostrophes, ß et Ł parmi ses lettres. Deux assertions qui figeaient l'ancien comportement
  changent : « ecrire » trouve « écrire », et une marque seule ne compte plus.
- **Plier aussi vite que mettre en minuscules** (`c0e3fc01`) : la recherche du calendrier dans
  25 000 rendez-vous dont toutes les notes ont le mot cherché prenait 0,94 s en debug (0,53 s avant
  le pliage), au bord de la seconde du test, que les machines de GitHub dépassent déjà. Les suites
  ASCII sont copiées d'un bloc, un mot sans partie ne fait pas de liste, et `quatrelle-model` se
  construit en `opt-level = 1` en debug (le `to_lowercase` de la bibliothèque standard qu'il
  remplace l'était toujours) : 0,20 s.
- **Tâches et personnes** (`d550a604`) : la recherche des tâches (objet, notes), celle des personnes
  des carnets (nom, organisation, adresses) et la liste du carnet d'adresses (adresses de
  l'utilisateur comprises) plient les deux côtés, toujours en sous-chaîne.
- **Défaut du fork trouvé par la mesure** (`c13a6ff5`) : la mise en texte du HTML coupe ses lignes
  (« Thank you for the\nupdate »), et une phrase n'y était pas trouvée, avant le pliage aussi. Une
  phrase se cherche maintenant dans le texte plié, chaque suite de blancs réduite à une espace.
- **Les mesures** (`96c489b8`, adapté de `db1bc510` ; `107ac7b4`, adapté de `19a5f797`) :
  `sample::big` (50 000 messages en 40 dossiers, la Boîte de réception 12 000, tous téléchargés, avec
  les réponses connues), écrit par lots ; `Shape` gagne `words` et `vocabulary` (par défaut ceux de
  l'origine : 40 mots tirés de 2 048). Le banc ignoré `bench::search::fifty_thousand` joue la
  fenêtre (6 ms par appel, 4 096 messages au plus) sur un fichier de données ; le test
  `ui-egui/src/tests/list/search_big.rs` met la boîte dans la fenêtre (verrou entre ses deux tests,
  durées vérifiées en `--release` seulement, 10 000 messages en debug). En `--release`, sur ce poste :

| 50 000 messages | textes courts (40 mots, 32 Mo de sources) | textes longs (600 mots sur 60 000, 228 Mo) |
|---|---|---|
| Première recherche, l'index bâti | 1,96 s de calcul en 348 appels (≈ 5,8 s d'images à 60 Hz) ; fenêtre : 294 images, p99 7,4 ms | 22,1 s en 4 762 appels (≈ 79 s à l'écran), pire appel 143 ms (la fin d'un dossier) |
| Entrées de l'index | 55 par message | 610 par message |
| Mémoire prise | +62 Mo (≈ 1,2 Ko par message) | +269 Mo (≈ 5,4 Ko par message, dont 15 Mo de texte gardé) |
| Recherche suivante (un mot, `zurich` pour Zürich) | ≤ 2,4 ms, un appel | ≤ 1,9 ms |
| Une phrase (16 667 corps relus) / `is:read` (35 000) | 0,12 s / 0,24 s | 1,13 s en 179 appels / 0,19 s |
| `search.idx` sur disque | 4,1 Mo | 53,7 Mo (≈ 1,1 Ko par message) |
| Après relancement, rien de changé | 0,11 s, 24 appels | 1,58 s, 41 appels, pire 89 ms (un dossier gardé lu d'un coup) |
| Après relancement, un message arrivé dans la Boîte de réception | 0,59 s, 119 appels | 6,9 s, 1 241 appels (≈ 21 s), toute la Boîte relue |

- **Pour P3, ramené à 550 000 éléments** (fois 11, profil des textes longs, le plus proche du
  propriétaire : la dev.3 a mis 6 à 7 minutes de calcul, ce profil en donne environ 4) : la première
  construction ≈ 4 min de calcul, ≈ 15 min avant que la recherche soit complète à l'écran (la
  fenêtre reste utilisable) ; **≈ 3 Go de mémoire** pour l'index (1,3 Go pour les seules listes de
  messages) ; ≈ 600 Mo de `search.idx`. Un relancement sans changement ≈ 17 s de calcul, mais **un
  dossier modifié depuis est relu en entier** : un dossier de 50 000 messages qui a reçu un message ≈
  30 s de calcul et plus d'une minute à l'écran ; et l'appel qui lit d'un coup un dossier gardé, ou
  qui finit d'en bâtir un (écriture de son `search.idx`, jonction), ≈ 0,4 à 0,6 s pour 50 000
  messages : un petit gel par dossier. Chez le propriétaire, la Boîte de réception, les Éléments
  envoyés et chaque dossier qui reçoit du courrier sont donc relus à chaque lancement.
- **Vérifié** : chaque sonde avant et après ; les suites de `quatrelle-model` (proptests compris),
  `quatrelle-engine` et `quatrelle-ui-egui` ; la cible de fuzz `search` compile (dictionnaire de
  l'origine et entrées des parties ; graines `folding` et `joined`) ; `cargo xtask ci` (wasm compris).
- **Où la recherche ne plie pas encore** : les suggestions de destinataires (`Contact::matches` et
  son nom en minuscules gardé, `mailboxes/people/suggest.rs`, `writing/names.rs` : classement,
  annuaire, salles, un ensemble à reprendre d'un bloc) ; les règles de courrier
  (`organize.rs`, `Condition::matches`, volontairement : elles suivent les règles du serveur) ; une
  phrase du calendrier coupée par un saut de ligne ; les recherches faites par les serveurs.
- **Reste, remarqué** : le vocabulaire fabriqué (des mots de six lettres) donne moins de mots
  différents que le vrai courrier, la mémoire des mots eux-mêmes est donc sous-estimée ; le pire
  appel dépasse la part de 6 ms à la fin de chaque dossier (déjà noté par R6), et `is:read` montre
  ses 35 000 trouvés en un appel de 32 à 63 ms.

### État de la récupération : v0.32, vingt-troisième partie : un rappel fermé ou reporté ici l'est aussi sur le serveur (2026-11-05)

Le lot R15 de `merge-tools/survey-v0.32.md` (issue #40 de l'origine, `0a6fc712`), sur `main`. Le
calendrier et ses rappels restent ceux du fork : la fonction de l'origine comble le manque noté au
relevé (`session/pim/reminders.rs` : fermer une occurrence de série et reporter restaient sur ce
poste, et Outlook les remontrait). Ce que le fork faisait déjà est gardé : fermer le rappel d'un
rendez-vous simple le coupe sur l'élément (`ReminderIsSet` sur EWS, `isReminderOn` sur Graph,
l'alarme retirée sur CalDAV), comme Outlook ; c'est aussi le repli de l'origine pour Exchange 2010,
donc ce repli n'est pas repris dans la couche réseau. Pour chaque protocole, dans l'ordre EWS (celui
du propriétaire), Graph, CalDAV : la sonde d'abord, lancée contre le code d'alors, sur les seuls faux
serveurs du dépôt.

| Sonde | avant | après |
|---|---|---|
| EWS, moteur : une occurrence de série fermée, un rendez-vous simple reporté (`a_dismissed_occurrence_and_a_snooze_go_to_exchange`) | aucune synchro demandée, rien gardé | synchro demandée ; chaque réponse gardée avec le calendrier (après un redémarrage aussi) jusqu'à la synchro qui la prend |
| EWS, faux serveur 2013 : un report simple, une occurrence fermée, un élément disparu (`reminders_answered_here_are_answered_on_exchange`) | aucun `GetReminders`, rien dit répondu | `Snooze` sur l'élément, `Dismiss` sur l'occurrence du jour (pas celle de la veille), le disparu laissé |
| EWS, faux serveur 2010 (`an_exchange_older_than_2013_is_not_asked`) | rien dit répondu | rien demandé au serveur, rien n'attend |
| EWS, serveur injoignable puis revenu (`an_answer_given_offline_waits_for_the_next_sync`) | rien envoyé | la réponse attend et l'erreur le dit ; la synchro suivante la donne |
| EWS, réponse `GetReminders` hostile : vide, coupée, 5 000 rappels, dates impossibles (`a_hostile_reminders_list_is_survived`) | rien demandé | aucune panique, réponse laissée |
| Graph, faux serveur (`reminders_answered_here_are_answered_on_graph`) | rien envoyé | `snoozeReminder` sur l'événement, `dismissReminder` sur l'occurrence trouvée parmi les instances, le disparu laissé |
| CalDAV, une occurrence de série fermée (`a_dismissal_on_caldav_acknowledges_the_alarm`) | événement inchangé | alarme `ACKNOWLEDGED`, événement envoyé, `SEQUENCE` inchangée ; l'occurrence du lendemain sonne ; un report reste ici |
| CalDAV, événement marqué ailleurs après son alarme (`an_alarm_acknowledged_elsewhere_isnt_due`) | dû ici | pas dû (marqué avant cette alarme : dû) |
| CalDAV, rendez-vous simple fermé (`a_single_dismissal_on_caldav_raises_no_sequence`) | `SEQUENCE` 2 → 3 | 2 |

- **EWS** (`20068408`, adapté de `0a6fc712`) : `Pim::reminder_answers` (500 au plus, gardées avec le
  calendrier ; plus de 30 jours après l'occurrence, elles s'en vont) et `DavSync::answered`, faites
  de réponses entières : une réponse redonnée pendant la synchro (reportée, puis fermée) attend
  encore, là où l'origine les reconnaissait par UID et occurrence. La synchro EWS, après la lecture
  du calendrier, demande `GetReminders` sous `RequestServerVersion` `Exchange2013` (de quatre
  semaines et un jour avant l'occurrence à un jour après), choisit le rappel par l'`ItemId` de
  l'élément, ou celui de sa série (`RecurringMasterItemId`), l'UID en secours, au début le plus
  proche à un jour près, puis `PerformReminderAction` (`Dismiss`, ou `Snooze` et `NewReminderTime`).
  Un rappel que le serveur ne liste plus, ou une opération refusée, est laissé (et consigné) ; le
  serveur injoignable ou occupé, cette réponse et les suivantes attendent la synchro d'après. **La
  version du serveur** : le fork ne la connaissait pas ; le client note le `MajorVersion` du
  `ServerVersionInfo` de chaque réponse, et sous 15 (Exchange 2010) rien n'est demandé. La réponse
  `GetReminders` est lue comme hostile (500 rappels, identifiants de 4 096 caractères au plus, dates
  vérifiées) et la cible de fuzz `text_first` la lit aussi (`reminders_listed`, `server_major`).
  Propre au fork : une fermeture de rendez-vous simple efface le report qui attendait pour lui. Le
  faux EWS répond aux deux opérations dans l'élément de réponse lui-même, comme Exchange, les
  refuse en 2010 (`ErrorInvalidServerVersion`), dit sa version (15 ou 14) dans chaque réponse et
  peut couper la connexion. Les 188 tests EWS d'avant passent avec cet en-tête.
- **Graph** (`4845b06a`, adapté de `0a6fc712`) : `dismissReminder` ou `snoozeReminder` sur
  l'événement, ou sur l'occurrence d'une série trouvée parmi ses `instances` autour de son début
  (50 au plus, à un jour près) ; un événement disparu (404) est laissé ; hors d'atteinte, on attend.
  Le moteur est le même que pour EWS (`an_answer_on_graph_waits_too`, garde-fou).
- **CalDAV** (`4b65b945`, adapté de `0a6fc712`) : selon la RFC 9074, l'occurrence fermée marque
  l'alarme de l'événement (l'exception de l'occurrence s'il en a une, sinon la série)
  `ACKNOWLEDGED` à l'heure de la fermeture, sans monter la `SEQUENCE`, et l'événement part au
  serveur ; un événement marqué ainsi par un autre programme après son alarme n'est pas dû ici,
  sauf reporté ici. Adapté : le fork lit une alarme lisible dans `CalEvent::reminder` et la
  réécrit, l'`ACKNOWLEDGED` serait donc perdu ; il est gardé à côté (`CalEvent::acknowledged`),
  relu et réécrit avec l'alarme (dont l'écriture passe dans `ical/alarms.rs`, `ical.rs` restant
  sous 800 lignes) ; une alarme gardée telle quelle reçoit le sien comme chez l'origine, une alarme
  de report (`RELTYPE=SNOOZE`) s'en va. Propres au fork : une invitation ne dit pas que
  l'organisateur a fermé son rappel ; un rappel remis sonne de nouveau. **Défaut du fork trouvé par
  la sonde** : fermer le rappel d'un rendez-vous simple passait par `update_event`, qui monte la
  `SEQUENCE` de ce que l'utilisateur organise ; un serveur CalDAV qui planifie pouvait y voir un
  changement de la réunion. Le rappel est maintenant coupé sans y toucher. Pris aussi : le test du
  modèle de l'origine (le reste de l'événement gardé, un événement abîmé survit), une propriété
  écrite ici (un rappel fermé à toute heure, écrit puis relu), et l'essai de l'origine contre un
  vrai serveur, porté en test ignoré `real_caldav_dismissed_reminder` (`QUATRELLE_REAL_CALDAV`,
  un Radicale local) : jamais lancé ici.
- **Pas pris** : le repli `ReminderIsSet` de la couche réseau de l'origine (le moteur du fork le
  fait déjà, pour tout serveur) ; le CHANGELOG (le fork n'en a pas).
- **Ce que verra le propriétaire** (Exchange sur site, 2013 ou plus récent) : une occurrence de
  réunion périodique fermée dans Quatrelle est fermée sur le serveur à la synchro, demandée tout de
  suite : Outlook, Outlook sur le web et le téléphone ne la remontrent plus (les rappels suivants
  de la série viennent). Un rappel reporté l'est sur le serveur à la même heure : tous les appareils
  le remontrent alors, ensemble. Une fermeture faite hors ligne attend la synchro suivante, même
  après un redémarrage. Le rendez-vous simple, comme avant : son rappel est coupé sur l'élément,
  partout. **Avec un Exchange 2010** : le rendez-vous simple aussi (ReminderIsSet) ; mais une
  occurrence de série fermée et un report restent sur ce poste, Outlook les remontrera.
- **Jamais essayé sur un vrai Exchange** : `GetReminders` et `PerformReminderAction` eux-mêmes
  (la forme des demandes, la fenêtre `BeginTime`/`EndTime`, que l'`ItemId` d'une occurrence et le
  `RecurringMasterItemId` y soient ceux que le fork garde, la présence de l'`UID`) ; la demande sous
  `Exchange2013` envoyée à un 2016 ou 2019 ; le vrai refus d'un 2010 (le fork s'en passe, la version
  suffit) ; `dismissReminder` et `snoozeReminder` de Graph ; le test Radicale.
- **Vérifié** : chaque sonde avant (échec) et après ; les suites de `quatrelle-model` (proptests
  compris), `quatrelle-engine` et les tests EWS et Graph de `quatrelle-net` ; `cargo xtask ci`
  (wasm compris), après une correction de clippy dans les tests (`6a0fd5d3`).
- **Reste, remarqué** : un rappel fermé dans Outlook pour une occurrence de série n'est pas lu ici
  (le fork calcule les rappels depuis les minutes ; il ne lit pas l'heure du prochain rappel du
  serveur) ; une exception déplacée de plus d'un jour n'est pas retrouvée parmi les rappels du
  serveur (sa réponse est laissée) ; l'alarme d'un rendez-vous est lue jusqu'à deux semaines
  (`duration_minutes`) quand le fork en permet quatre (`REMINDER_MAX_MINUTES`) : au-delà elle reste
  entière parmi les lignes gardées, toujours lue, la propriété l'a trouvé ; dans les tests du
  moteur, une session rouverte ne retrouve pas les rappels répondus sur ce poste (les préférences
  sont enregistrées par la fenêtre, non vérifié ici).

### État de la récupération : v0.32, vingt-quatrième partie : un suivi marqué terminé le reste, avec son jour, et va jusqu'à Exchange (2026-11-05)

Le lot R16 de `merge-tools/survey-v0.32.md` (`f1eb8a31`, `39d8684f`, et la correspondance EWS que
l'origine n'a pas), sur `main` ; il clôt la phase 3. « Marquer comme terminée » retirait le drapeau
et oubliait le jour d'échéance : le message avait l'air de n'avoir jamais été suivi, et Annuler
perdait l'échéance. Comme Outlook, il reste maintenant marqué terminé, avec le jour où il l'a été.
La marque est le mot-clé `$FollowUpComplete` de l'origine ; chaque serveur la garde à sa façon :
Exchange dans son état de drapeau (EWS `PidTagFlagStatus` = 1, Graph `flagStatus` « complete »),
IMAP et JMAP comme mot-clé. Le jour est gardé dans les préférences (`followUpDone`, comme chez
l'origine), sauf qu'Exchange garde le sien (`PidTagFlagCompleteTime`, `completedDateTime`), lu dans
`Flags::completed` (secondes Unix ; propre au fork), qui l'emporte. Les fichiers de l'origine
`folder_edits.rs` et `summary.rs` sont chez nous `session/mailboxes/folder_edits.rs` et
`model/src/lib.rs` ; le code du suivi part dans `session/mailboxes/follow_up.rs` (nouveau). Pour
chaque partie, la sonde d'abord, lancée contre le code d'alors, sur les seuls faux serveurs du dépôt.

| Sonde | avant | après |
|---|---|---|
| Moteur : marquer terminé, `is:complete`, Annuler (`probe_mark_complete_keeps_it_complete`, puis les trois tests de l'origine dans `engine/src/tests/mail/follow_up.rs`) | message sans aucune marque (`keywords: []`) ; à la lecture du code, Annuler remettait le drapeau sans l'échéance | marqué terminé, fait aujourd'hui, trouvé par `is:complete`, rangé sous « Completed » ; Annuler remet drapeau, échéance et jour en un pas |
| Modèle : `is:complete`, `is:completed`, `-is:complete`, drapeau remis ailleurs (`search::tests::matching`) | rien trouvé (lu comme du texte) | trouvé ; un message de nouveau marqué ailleurs, le mot-clé resté, ne l'est pas |
| Modèle : `day_label(i64::MAX)` (jour venu d'un fichier abîmé) | **panique** (débordement, en debug) | une étiquette |
| Moteur : jours lus du disque hors bornes, clés vides ou trop longues (`the_preferences_days_read_from_disk_are_days`) | — (pas de `followUpDone`) | laissés de côté |
| IMAP et JMAP : la requête envoyée pour `is:complete` | aucune | `KEYWORD $FollowUpComplete UNFLAGGED` ; `hasKeyword` + `notKeyword $flagged` ; nié : laissé à la liste |
| IMAP, faux serveur : le mot-clé posé puis retiré dans l'ordre du moteur (`keywords_round_trip_with_the_server`) | **passait déjà** (mots-clés génériques) | gardé comme garde-fou |
| EWS, faux serveur : un message terminé dans Outlook (`0x1090` = 1, `0x1091`), marquer terminé ici, Annuler, retirer, puis changements faits dans Outlook (`a_follow_up_marked_complete_goes_both_ways_with_its_day`) | arrivé **non marqué, sans jour** | lu terminé à son heure ; écrit 1 et l'heure ; 2 sans l'heure ; ni l'un ni l'autre ; la synchro suivante (seulement ce qui a changé) suit Outlook |
| EWS : réponses hostiles (état 7, heure illisible, avant 1970, heure sans état) (`a_follow_up_read_from_exchange_is_read_with_care`) | non lu | lu avec soin, rien gardé de faux |
| Cache : marque en place de R10 (`state-marks.json`) et réouverture (`a_completed_follow_up_and_its_time_survive_the_marks`) | **passait déjà** (les marques gardent tout `Flags`) | garde-fou ; le mot-clé retiré ici emporte l'heure |
| Graph, faux serveur : terminé dans Outlook, marquer terminé ici, Annuler, drapeau remis dans Outlook (`a_follow_up_marked_complete_goes_both_ways`) | arrivé **non marqué** | `complete` dans les deux sens, `completedDateTime` lu |

- **Moteur, modèle, recherche** (`ffa89626`, adapté de `f1eb8a3`) : `FOLLOW_UP_COMPLETE`,
  `Flags::follow_up_complete()` (le mot-clé et pas de drapeau : un drapeau remis ailleurs l'emporte),
  `Flags::completed` (remis à zéro dès que le mot-clé change ici), `Term::Complete`, `Undo::FollowUp`
  (drapeau, mot-clé et les deux jours d'un coup), `Session::complete_selection` et
  `follow_up_done` ; rangé par drapeau : « Flagged », puis « Completed », puis « Not flagged » ; la
  ligne de commande montre `✓`. Ordre des changements, qui fait finir juste Exchange : pour terminer,
  le drapeau ôté puis le mot-clé posé ; pour Annuler ou remettre le drapeau, le mot-clé ôté puis le
  drapeau posé. Les jours lus du disque sont vérifiés (de 1601 à 9999, 10 000 au plus, clé de 998
  caractères au plus) ; `day_label` ne déborde plus. Adapté : pas de `MAX_FOLLOW_UP_ID` ni de
  `merge_follow_up` chez nous (écrits ici), pas de `is_system_keyword` (le fork traite déjà les
  mots-clés en `$` comme n'étant pas des catégories).
- **EWS** (`3a262e87`, propre au fork) : chaque demande des marques d'un élément demande aussi
  `0x1091` (`FLAG_FIELDS`, donc la synchro complète, `SyncFolderItems`, `GetItem` et la recherche) ;
  l'état 1 est lu `$FollowUpComplete`, l'heure `Flags::completed` (une heure illisible ou d'avant 1970
  laissée) ; `take_marks` prend les deux du serveur, un mot-clé resté ici compris. Écrit : terminé =
  état 1, l'heure de maintenant, et l'icône de drapeau d'Outlook (`0x1095`) effacée comme le dit
  [MS-OXOFLAG] 3.1.4.2 ; drapeau remis = état 2, l'heure effacée ; drapeau ôté ou mot-clé ôté = les
  deux effacés. Le faux EWS garde l'état et l'heure (et les rend dans ses états de synchro) ; la cible
  de fuzz `text_first` lit les marques de chaque message (`marks_listed`). Les 190 tests EWS passent.
- **Graph** (`9752537b`, adapté de `39d8684`) : `flag_patch` de l'origine ; `completedDateTime` lu
  (demandé en UTC par l'en-tête `Prefer` du fork ; une autre zone est laissée), ce que l'origine ne
  faisait pas encore. L'écriture reste celle de l'origine : `flagStatus` seul, sans
  `completedDateTime`.
- **Interface** (`75d7c1e2`, adapté de `f1eb8a3`) : à la place du drapeau, une coche, l'icône Fluent
  `checkmark_20_regular.svg` (ajoutée, ligne dans `ATTRIBUTION.md` ; `Icon::Done`, trait de secours
  dessiné), dans la liste et dans la colonne du drapeau, en couleur atténuée ; cliquer la coche fait
  ce que fait le clic sur le drapeau du fork, le message redevient marqué (comme Outlook) ; le
  lecteur d'écran dit « Completed » ; le volet de lecture dit « Terminé le jeu. 2026-11-05. », là où
  il dit l'échéance. Deux textes nouveaux, dans les 13 catalogues (traductions de l'origine). Le test
  sans tête `ui-egui/src/tests/list/follow_up.rs` (le dossier `tests/` étant plein) vérifie la ligne,
  le volet, le titre et « Effacer l'indicateur » ; avec `QUATRELLE_FRAME_PNG` il peint aussi ses
  images (un petit peintre logiciel des triangles d'egui, pour les regarder sans lancer
  l'application) : clair, sombre, rangé par drapeau, en colonnes, regardées.
- **Pas pris** : le CHANGELOG (le fork n'en a pas) ; la recherche serveur EWS et Graph de
  `is:complete` (laissée à la liste, comme `is:pinned`).
- **Ce que verra le propriétaire** : « Marquer comme terminée » laisse une coche à la place du
  drapeau, le volet dit « Terminé le … » ; Annuler remet le drapeau et son échéance ; un clic sur la
  coche remet le drapeau. Dans Outlook (même compte Exchange sur site), le message marqué terminé
  ici apparaît terminé (coche) après sa synchro ; un message marqué terminé dans Outlook arrive ici
  avec sa coche et le jour qu'Outlook a noté. Un drapeau remis dans Outlook efface la coche ici.
  **Mais** le courrier déjà en cache et terminé dans Outlook avant cette version reste sans coche :
  la synchro EWS ne demande que ce qui a changé (`SyncFolderItems`) ; il ne la prend qu'au premier
  changement du message ou quand un dossier est relu en entier (état de synchro refusé, dossier
  trop changeant). Forcer une relecture complète des marques une fois coûterait, sur 550 000
  éléments, une lecture entière de chaque dossier : à décider avec le propriétaire.
- **Jamais essayé sur un vrai Exchange** : l'écriture de `0x1090` = 1 avec `0x1091` en
  `SystemTime` et l'effacement de `0x1095` (qu'Outlook montre bien la coche et la date sans les
  propriétés de tâche, `PidLidTaskComplete`, `PidLidPercentComplete`, `PidLidTaskDateCompleted`, que
  le fork n'écrit pas) ; la forme exacte de l'heure rendue par Exchange ; `flagStatus` « complete »
  sur Graph, et si Exchange Online remplit seul `completedDateTime`.
- **Vérifié** : chaque sonde avant et après ; les suites de `quatrelle-model`, `quatrelle-engine`,
  `quatrelle-store`, `quatrelle-jmap`, `quatrelle-ui-egui` et les tests EWS et Graph de
  `quatrelle-net` ; `cargo xtask ci` (wasm compris).
- **Reste, remarqué** : l'import `.pst` ne lit encore que l'état 2 (`data_files/pst.rs`) : un message
  terminé dans un fichier d'Outlook arrive non marqué (à faire avec P2) ; le fork n'écrit pas
  non plus l'icône `0x1095` en posant un drapeau, ni les propriétés de tâche d'Outlook ; un message
  terminé hors ligne reçoit sur Exchange l'heure de l'envoi du changement, non celle du clic (le jour
  gardé ici reste juste jusqu'à ce que le serveur dise le sien).

### Hors relevé : R17, les coches d'avant et l'index par ajout (2026-11-05)

Un petit lot propre au fork, décidé par le propriétaire le 05/11 (phase 4 de
`merge-tools/survey-v0.32.md`), en deux parties. Pour chacune, la sonde d'abord, lancée contre le
code d'alors, sur les seuls faux serveurs et boîtes fabriquées du dépôt.

**Première partie : la coche des suivis terminés avant R16** (`b19e997e`). Un message marqué
terminé dans Outlook avant cette version restait sans coche : la synchro EWS ne demande que ce qui
a changé. Décision : une passe ciblée, une seule fois.

| Sonde | avant | après |
|---|---|---|
| EWS, faux serveur : cache « de la version d'avant » (coche ôtée, état sans passe), Boîte de réception (1 terminé) et un dossier (3 terminés), synchro ordinaire (`follow_ups_completed_before_this_version_are_ticked_once`) | **aucune coche** | la première synchro met coche et jour d'Outlook ; une requête pour la Boîte de réception, deux pages pour l'autre dossier (page de 2 en test) ; passe coupée après la Boîte de réception puis reprise : elle n'est pas redemandée ; passe finie : plus rien demandé |
| EWS : dossier d'une boîte partagée et dossier public suivi (`shared_and_public_folders_have_their_ticks_read_where_they_live`) | — | cochés, une requête chacun, là où ils vivent (ancrée à la boîte, routée vers la boîte de contenu public) |
| Graph : même cache sans coche, synchro ordinaire (`a_follow_up_completed_before_this_version_is_ticked_at_the_next_sync`) | **passait déjà** | garde-fou |
| État du compte : lecteurs parallèles, fin de passe, `jmap.json` ancien ou abîmé (`the_completed_follow_ups_pass_is_kept_per_account`) | — | dossiers faits rendus par `absorb` ; fin quand tous les dossiers suivis l'ont eue ; un `ticks` illisible refait la passe sans refuser l'état (ni sa numérotation) |

- **EWS** : au premier passage de chaque dossier de courrier suivi dans une synchro (avant même le
  retour anticipé d'un dossier inchangé), un `FindItem` `IdOnly` avec `FLAG_FIELDS` et la
  restriction `0x1090 = 1` (celle du drapeau, `= 2`, en modèle), paginé par 1 000, limité à la
  fenêtre et à l'âge gardé (`Client::completed`, `newest_where` prenant maintenant une condition) ;
  les messages que le cache connaît prennent les marques du serveur (`take_marks`, le jour
  `0x1091` compris), rien d'autre n'est lu (`ews/sync/catch_up.rs`, nouveau, avec ce que partage le
  rattrapage des catégories). Un dossier sans cache n'a rien à cocher (il sera listé en entier) ;
  un dossier public non suivi n'est pas concerné. Un refus du serveur (`NetError::No`) est consigné
  et compte comme fait ; hors ligne, l'erreur remonte et le dossier est repris à la synchro suivante.
- **La marque** est dans l'état du compte (`jmap.json`, champ `ticks` : les dossiers faits, puis
  `done`), pas dans `ui.json` ; `sync_all` la clôt quand tous les dossiers suivis l'ont eue. Un
  lecteur parallèle part d'une copie et rend ses dossiers (`absorb`). Le champ est lu à part : abîmé,
  il est pris pour « passe à faire », sans refuser le reste de l'état.
- **Graph** : rien à faire. Sa synchro relit à chaque fois les marques (`flag`) de toute la fenêtre
  gardée, et R16 y lit `complete` : la coche arrive à la première synchro (sonde).
- **IMAP et JMAP** : rien à faire. Ils gardaient déjà tous les mots-clés tels quels avant R16
  (`$FollowUpComplete` compris, comparé sans casse) : le cache a la marque, R16 sait la lire (la
  sonde IMAP de R16 passait déjà).
- **Le faux EWS** distingue `0x1090 = 1` de `= 2` dans une restriction et compte ces demandes à part.

**Deuxième partie : un message reçu ne fait plus relire tout l'index de son dossier**
(`998e7e40`). R14 l'avait mesuré : après un relancement, un dossier changé était relu en entier.
Décision : pas de portage de l'index de l'origine (P3), seulement cela.

| Sonde | avant | après |
|---|---|---|
| Moteur, 300 + 10 messages téléchargés, l'index bâti, un message reçu, relancement, recherche (`a_change_reads_the_message_changed_not_its_folder`) | **301 sources relues** | 1 au plus ; puis un retiré, un déplacé, un résumé changé (aperçu rempli) et un corps arrivé : 3 au plus ; la fois d'après : 0 ; mêmes trouvés |
| Proptest : ajouts, retraits, déplacements, résumés changés, corps arrivés au hasard, puis l'index repris comparé à un index rebâti, et l'index qu'il garde à son tour (`a_kept_index_taken_with_its_changes_finds_what_a_new_one_does`, 40 cas) | — | identiques ; somme ignorée exprès : il échoue |
| En session, un message reçu puis la recherche suivante (banc) | 1 appel, 0 source relue | inchangé (déjà par ajout, `top_up`) |

Le banc ignoré `bench::search::fifty_thousand` (`--release`, sur ce poste) compte maintenant les
sources relues et ajoute le cas en session, puis un retrait et un déplacement. Textes longs (600 mots
sur 60 000, le profil le plus proche du propriétaire) :

| | avant, 50 000 messages | après, 50 000 | après, 45 000 |
|---|---|---|---|
| Relancement, rien de changé | 41 appels, 1,62 s, pire 90 ms | 41 appels, 1,57 s, pire 91 ms | 41 appels, 1,39 s, pire 79 ms |
| Relancement, un message arrivé dans la Boîte de réception | 1 241 appels (≈ 21 s), 6,87 s, pire 152 ms, **12 002 sources relues** | 77 appels, 1,93 s, pire 109 ms, **2 relues** | 43 appels, 1,52 s, pire 94 ms, 2 relues |
| Relancement, un retiré de la Boîte de réception, un déplacé vers un autre dossier | 1 035 appels, 6,09 s, pire 131 ms, 12 976 relues | 78 appels, 1,96 s, pire 113 ms, 1 relue | 45 appels, 1,57 s, pire 93 ms, 1 relue |
| Première recherche, pire appel (une fin de dossier) | 132 à 152 ms | 119 à 130 ms | 109 ms |

Textes courts, 50 000 : un message arrivé, 112 appels, 0,53 s, 12 002 relues avant ; 61 appels,
0,32 s, 2 relues après. **Le banc à 50 000 tout juste** dépasse de deux messages le cache des listes
de la session (`CACHE_MESSAGES`, 50 000 résumés) : chaque dossier relu en chasse un autre et la
réponse relit les listes sur le disque (2,6 ms chacune), d'où l'écart avec 45 000. C'est un artefact
de la taille du banc, pas de la reprise.

- **`search.idx` version 4** (`model/src/search/index.rs`) : chaque message garde sa somme
  (`summary_sum`, FNV de ce que `summary_text` donne à l'index, numéro compris ; 8 octets de plus
  par message). `FolderIndex::retain` retire des messages et renumérote en une passe
  (`Index::renumber`, l'ordre gardé, les listes jamais retriées). Un fichier abîmé ou d'une autre
  version est reconstruit ; **un fichier de version 3 est encore lu**, mais pris seulement pour un
  dossier inchangé (ses sommes alors calculées, le fichier réécrit) : la mise à jour ne reconstruit
  rien de ce qui n'a pas changé.
- **Au relancement** (`session/searching/indexing/kept.rs`, le code des dossiers gardés déplacé là
  par sujet) : un dossier changé n'est plus jeté. Les messages disparus ou dont la somme a changé
  sortent, les autres restent ; ceux qui manquent sont lus comme un dossier en cours, les présents
  passés (1/64 d'unité chacun), puis le fichier est réécrit à la fin du dossier. Les corps arrivés
  depuis sont lus comme avant ; le déplacement est un retrait ici et un ajout là-bas.
- **En session**, rien ne change : un ajout, un corps arrivé (`Event::Body`, les sources de R14
  `came`) vont déjà par ajout en mémoire ; un message parti ne répond plus (sa place est vérifiée).
  `search.idx` n'est pas réécrit en cours de session : le relancement suivant reprend ces
  changements, au prorata (réécrire le fichier d'un dossier demanderait d'extraire ses mots de
  l'index de toute la boîte).
- **L'appel qui finit un dossier** écrit son fichier, et la jonction se fait à l'appel suivant
  (`joining`) ; un dossier repris avec des changements, sans rien à lire, passe aussi par là (sinon
  lecture, retrait et écriture faisaient 164 ms d'un appel). Un dossier repris entier est joint tout
  de suite : différé, il doublait les appels d'un relancement (41 → 81) pour 14 ms de moins au pire.
  Le test `paced` exclut maintenant les deux derniers appels (écriture 21 ms, jonction 1,6 ms en
  debug, au lieu de 45 ms d'un tenant).
- **Essayé puis retiré** : garder d'un appel à l'autre la table des places (où est chaque message
  dans sa liste) : aucun gain mesuré (0,3 ms par appel), et de l'ordre de 20 Mo chez le propriétaire.
- **Vérifié** : chaque sonde avant (échec) et après ; le proptest échoue si la somme est ignorée ;
  les suites de `quatrelle-model`, `quatrelle-store`, `quatrelle-engine`, `quatrelle-jmap` et
  `quatrelle-net` ; la cible de fuzz `search` lit aussi `search.idx` (lecture, retrait, réécriture) ;
  `cargo xtask ci` (wasm compris).
- **Ce que verra le propriétaire** au premier démarrage :
  - **les coches** : à la première synchro complète, chacun de ses dossiers de courrier gardés
    (110, boîte partagée et dossiers publics suivis compris) reçoit **une requête de plus**, une
    page de plus par millier de messages terminés dans un dossier ; un dossier public coûte en plus
    l'essai non routé que fait toute requête de ces dossiers. Soit 110 à 130 petites requêtes
    `IdOnly`, une fois, mêlées à la synchro. Les messages terminés dans Outlook prennent leur coche
    et « Terminé le … » ; si Quatrelle est fermée en route, la synchro suivante finit les dossiers
    restants ;
  - **la recherche** : ses `search.idx` de version 3 des dossiers inchangés sont repris sans
    relecture, mais réécrits une fois chacun (≈ 600 Mo en tout, un fichier par appel) ; les
    dossiers changés depuis leur dernier index (la Boîte de réception, les Éléments envoyés…) sont
    relus une dernière fois en entier. Ensuite, un relancement ne relit que les messages arrivés,
    changés ou déplacés : quelques secondes de moins par gros dossier, une minute de moins pour un
    dossier de 50 000 messages, selon les mesures de R14.
- **Reste, remarqué** : lire un gros `search.idx` reste un appel d'un tenant (lecture 28 ms et
  analyse 35 ms pour 10 800 messages longs, de l'ordre de 0,3 s pour 50 000), comme l'écrire
  (85 ms pour 10 800) et le joindre ; le démarrage (tous les `search.idx` relus, ≈ 1,5 s de calcul
  pour 50 000) n'est pas raccourci ; chez le propriétaire, une recherche dans toute la boîte dépasse
  toujours le cache de 50 000 résumés, et la réponse relit alors des listes sur le disque.
- **Jamais essayé sur un vrai Exchange** : le `FindItem` avec la restriction `0x1090 = 1`, sa
  pagination, son routage vers les dossiers publics et la boîte partagée.

### Hors relevé : R18, répondu et transféré comme Outlook (2026-11-05)

Un lot propre au fork, demandé par le propriétaire le 05/11 : la flèche d'un message auquel il a
répondu (vers la gauche) ou qu'il a transféré (vers la droite), et la ligne du volet « Vous avez
répondu à ce message le … ». La liste dessinait déjà la flèche d'après `\Answered` et `$Forwarded`,
qu'IMAP et JMAP lisent et écrivent, et le moteur posait la marque sur l'original après l'envoi.
Mais EWS ne lisait ni n'écrivait l'état : Outlook le garde dans `PidTagLastVerbExecuted` (`0x1081` :
102 répondu à l'expéditeur, 103 à tous, 104 transféré) et `PidTagLastVerbExecutionTime` (`0x1082`),
et le fork envoie son propre message (`CreateItem` avec la source), si bien qu'Exchange ne marque
rien de lui-même. Graph de même (`sendMail` avec la source). Pour chaque partie, la sonde d'abord,
lancée contre le code d'alors, sur les seuls faux serveurs du dépôt.

| Sonde | avant | après |
|---|---|---|
| EWS, faux serveur : répondu, répondu à tous, transféré dans Outlook (`0x1081`, `0x1082`), répondre puis transférer ici, puis répondre dans Outlook (`a_reply_or_a_forward_goes_both_ways_with_its_time`) | arrivés **sans flèche ni heure** (`(false, false, 0)`) ; écrit ici : **rien** (aucun `UpdateItem`) | flèche et heure d'Outlook ; écrit 102 (ou 104), l'heure et l'icône 261 (262) ; la synchro suivante garde la flèche, la dernière action l'emporte ; la réponse faite dans Outlook arrive |
| EWS : flèche posée ici, puis un changement dans Outlook, puis le dossier relu en entier (`an_arrow_kept_here_survives_the_syncs_that_say_nothing_of_it`) | **passait déjà** (`take_marks` gardait la marque) | garde-fou : une synchro qui ne dit rien de la dernière action ne l'ôte pas |
| EWS : valeurs hostiles (vote 261, 105, « abc », nombre géant, heure illisible, d'avant 1970, l'« aucune » de MAPI en 4501, heure sans verbe, étiquette en décimal) (`a_reply_read_from_exchange_is_read_with_care`) | non lu | lu avec soin, rien gardé de faux |
| Moteur, compte Exchange : répondre, transférer, répondre à tous depuis la Boîte d'envoi (`on_exchange_the_last_reply_or_forward_shows`) | après le transfert, **les deux flèches** | la dernière seule, tout de suite ; l'autre ôtée d'abord (changement sans effet sur le serveur) |
| Moteur, compte IMAP : répondre puis transférer (`over_imap_a_reply_and_a_forward_both_stay`) | **passait déjà** | garde-fou : les deux marques restent, faits distincts |
| Graph, faux serveur : répondu dans Outlook, cache « d'avant », transférer ici, répondre à tous dans Outlook (`a_reply_or_a_forward_goes_both_ways_over_graph`) | arrivé **sans flèche** | flèche et heure, à la première synchro comme à la suivante d'un cache d'avant ; écrit 104, l'heure et 262 |
| EWS, passe unique : cache « d'avant » (flèches ôtées, état sans la passe), Boîte de réception (1 répondu, 1 voté) et un dossier (3 : répondu, à tous, transféré), passe coupée puis reprise (`replies_made_before_this_version_get_their_arrow_once`) | **aucune flèche** après la synchro | une requête pour la Boîte de réception, deux pages pour l'autre dossier (page de 2 en test) ; le vote n'est pas demandé ; reprise sans refaire la Boîte de réception ; passe finie : plus rien demandé |
| EWS : boîte partagée et dossier public suivi (garde-fou de R17 étendu) | — | une requête de plus chacun, là où ils vivent |
| État du compte : lecteur parallèle, fin de passe, `replies` abîmé (`the_completed_follow_ups_pass_is_kept_per_account`, étendu) | — | dossiers rendus par `absorb`, passe finie, une marque abîmée refait la passe sans toucher à l'autre |
| Interface sans tête : liste et volet pour un message répondu (heure d'Exchange), transféré, répondu sans heure (IMAP), et un sans rien (`tests/list/replied.rs`) | — (ligne absente du volet) | « Replied »/« Forwarded » lus à l'écran, les trois lignes du volet, aucune pour le dernier ; clair et sombre regardés |

- **Modèle et moteur** (`6c6ebf46`) : `Flags::acted` (secondes Unix ; 0 : non dit), l'heure de la
  dernière action dite par le serveur, oubliée dès qu'une des deux marques change ici (comme
  `Flags::completed` de R16). `Session::mark_original` pose la marque sur l'original après la
  remise (directe ou depuis la Boîte d'envoi) ; sur un compte Exchange (EWS ou Graph) il ôte
  d'abord l'autre flèche, car Exchange ne garde que la dernière action et Outlook ne montre qu'elle.
- **EWS** (`6c6ebf46`) : `FLAG_FIELDS` demande `0x1081` et `0x1082` (donc la synchro complète,
  `SyncFolderItems`, `GetItem`, la recherche et les passes) ; 102 et 103 sont lus répondu, 104
  transféré, un autre verbe (vote) ni l'un ni l'autre ; l'heure est gardée si elle est lisible,
  après 1970 et avant 4501. `take_marks` prend la dernière action **dès que le serveur en dit une** ;
  sinon la flèche gardée ici reste (réponse faite par une version qui ne disait rien à Exchange).
  Écrit, comme Outlook le fait sur l'original ([MS-OXOMSG] `PidTagLastVerbExecuted`,
  `PidTagLastVerbExecutionTime`, `PidTagIconIndex`) : le verbe, l'heure d'envoi du changement et
  l'icône 0x105 (261) ou 0x106 (262). **Répondre à tous est écrit 102** : le changement du moteur
  (`Op::SetFlag`) ne dit pas le mode, et Outlook montre l'un et l'autre de même. Ôter une flèche
  n'écrit rien. Le faux EWS garde le verbe, l'heure et l'icône (éléments, états de synchro,
  restriction par plage) ; la cible de fuzz `text_first` lit déjà les marques de chaque message.
- **Graph** (`bcb9c34e`) : sûr, donc fait. Graph n'a pas de champ pour cela, mais donne et prend
  les propriétés d'Outlook (`singleValueExtendedProperties`). Chaque liste d'un dossier, les
  messages retrouvés hors fenêtre et les détails d'un message développent `Integer 0x1081` et
  `SystemTime 0x1082` (un seul `$expand` avec le filtre des boutons de vote, joints par `or`) ;
  écrit par `PATCH` : le verbe, l'heure et `Integer 0x1080`. La liste d'un dossier relit à chaque
  synchro les marques de toute la fenêtre gardée : pas de passe pour Graph.
- **Interface** (`44d65396`, textes repris aussitôt après) : au volet, au-dessus du message, avec la
  flèche, « Répondu le jeu. 2026-11-05 10:12. » ou « Transféré le … » (jour et heure), sur le modèle
  de « Terminé le … » de R16 ; sans heure connue (IMAP, JMAP, Exchange pas encore relu) « Répondu »
  ou « Transféré ». Les deux marques gardées (IMAP) : la réponse, comme la liste. La ligne de la liste dit
  « Répondu » ou « Transféré » au lecteur d'écran. **La flèche de la liste passe de `t.faint` à
  `t.dim`** (la teinte du trombone et de la coche de R16) : en sombre, sur la ligne choisie, elle se
  voyait à peine (rendus regardés, gardés hors du dépôt dans `~/.tq/r18-shots/`). Six textes dans
  les 13 catalogues au départ, quatre ensuite (`Replied on {date}.`, `Forwarded on {date}.`, et les
  mots `Replied` et `Forwarded` déjà là) ; le peintre d'images du test de R16 est partagé dans
  `tests/list/`.
- **Passe unique** (`d9c1b1db`) : sur le modèle de R17, au premier passage de chaque dossier de
  courrier suivi, un `FindItem` `IdOnly` avec `FLAG_FIELDS` et la restriction `0x1081 ≥ 102` et
  `≤ 104`, paginé par 1 000, limité à la fenêtre et à l'âge gardé (`Client::replied`) ; les
  messages que le cache connaît prennent la flèche et l'heure (`take_marks`), rien d'autre n'est
  lu. Marque dans `jmap.json`, champ `replies` à côté de `ticks` (même type, `Ticks`, dont les
  règles passent sur lui : `wanted`, `read`, `finish`, `absorb`) ; les deux passes partent d'un
  seul appel, `catch_up_once`, et `sync_all` clôt chacune quand tous les dossiers suivis l'ont eue.
  Un refus du serveur (`NetError::No`) compte comme fait ; hors ligne, le dossier est repris.
- **Ce que verra le propriétaire** :
  - **dans Quatrelle** : la flèche à gauche ou à droite dans la liste, de la teinte du trombone ;
    au volet « Répondu le … » ou « Transféré le … » ;
    sur Exchange, la dernière action l'emporte (répondu puis transféré : la flèche de transfert),
    tout de suite après l'envoi ; Gmail et Yahoo (IMAP) gardent les deux marques et montrent la
    réponse, sans heure ;
  - **dans Outlook** (même compte Exchange sur site) : un message répondu ou transféré depuis
    Quatrelle montre, après sa synchro, la flèche et le bandeau « Vous avez répondu … », à l'heure
    où Quatrelle a envoyé le changement ; ce qu'il a fait dans Outlook, avant comme après cette
    version, arrive ici avec sa flèche et son heure ;
  - **au premier démarrage** : la passe, mêlée à la première synchro complète.
- **Coût de la passe sur sa boîte** (550 000 éléments, 110 dossiers et quelques dossiers suivis de
  la boîte partagée et des dossiers publics, mettons 10 à 20 % répondus ou transférés, soit 55 000 à
  110 000 messages) : **une requête par dossier suivi** (110 à 130, comme R17), **plus une page par
  millier de messages trouvés au-delà du premier** (55 à 110 pages), soit **de l'ordre de 170 à
  240 requêtes `FindItem`, une seule fois**. Chaque message rendu pèse de l'ordre de 0,6 à 0,8 Ko de
  XML (identifiant, clé de changement, lu, drapeau, verbe, heures, catégories) : 35 à 90 Mo reçus en
  tout. Côté Exchange, la restriction porte sur une propriété non indexée : le serveur parcourt
  chaque dossier (quelques secondes pour un dossier de 50 000 éléments, peut-être plus la première
  fois) ; à 1 à 3 s par page, **quelques minutes de synchro en plus**, une fois. Ici, chaque dossier
  touché voit ses pages de liste réécrites pour les messages changés (`save_folder_changed`), une
  fois ; la mémoire tient la liste des identifiants trouvés d'un dossier (≈ 15 Mo pour 50 000). La
  fenêtre et l'âge gardés (le « courrier à garder hors connexion ») bornent tout cela. Coupée
  (fermeture, perte du réseau), la passe reprend au dossier suivant.
- **Jamais essayé sur un vrai Exchange** : la restriction par plage sur `0x1081` (`IsGreaterThanOrEqualTo`
  et `IsLessThanOrEqualTo` sous un `And`), sa durée sur un gros dossier et son routage vers la boîte
  partagée et les dossiers publics ; l'écriture de `0x1081`, `0x1082` et `0x1080` par `UpdateItem`
  (élément écrit `<t:Message>`, comme le drapeau : une invitation pourrait le refuser) et qu'Outlook
  montre alors flèche et bandeau ; la forme exacte de l'heure rendue ; sur Graph, le `$expand` à trois
  propriétés jointes par `or`, son coût sur une liste de toute la fenêtre à chaque synchro, et le
  `PATCH` des trois propriétés.
- **Vérifié** : chaque sonde avant (échec, ou déjà juste pour les deux garde-fous) et après ; les
  suites de `quatrelle-model`, `quatrelle-engine`, `quatrelle-jmap`, `quatrelle-net` et
  `quatrelle-ui-egui` ; `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - la vue en colonnes (volet de lecture fermé) n'a pas de colonne d'icône : pas de flèche là ;
  - répondre à tous est écrit 102, non 103 (il faudrait que `Op::SetFlag` porte le mode) ;
  - les messages auxquels il a répondu **depuis Quatrelle** avant cette version, sur Exchange,
    gardent leur flèche ici mais Exchange n'en sait rien : Outlook ne la montre pas ; aucune passe
    ne les lui écrit ;
  - IMAP n'a pas d'ordre entre `\Answered` et `$Forwarded` : la réponse est montrée ; un message
    voté puis répondu dans Outlook montre la réponse, un message répondu puis voté garde sa flèche
    ici (Outlook dessine alors l'icône de `0x1080`, que le fork ne lit pas) ;
  - **textes repris** : la consigne demandait les phrases françaises d'Outlook (« Vous avez répondu
    à ce message le … ») ; l'en-tête des catalogues dit qu'aucun texte n'est repris d'un autre client,
    et `CLAUDE.md` limite les mots d'Outlook aux noms de fonctions. Elles sont remplacées, avant
    l'envoi, par les tournures propres à Quatrelle ci-dessus, dans les 13 langues ;
  - l'import `.pst` ne lit pas `0x1081` (à faire avec P2, comme l'état 1 du drapeau).

### État de la récupération : v0.32, vingt-cinquième partie : l'import fidèle des fichiers .pst, le courrier (2026-11-05)

La première moitié du lot P2 de `merge-tools/survey-v0.32.md` (P2a, décision n° 7 du propriétaire :
« en entier »), sur `main`, plus les restes de R16 et R18. L'import du fork reste le sien (un fil,
un passage, pas de plan, pas de reprise) : rien de l'import suivi et reprenable de l'origine (C1-4),
ni de ses copies à côté des `.pst`, ni de son index sur disque. Pour chaque commit, son test porté
seul d'abord (ou une sonde écrite ici quand le test ne compilait pas contre le code d'alors), lancé
contre le code d'alors, sur des fichiers fabriqués par l'écrivain du crate (`outlook-pst-rw`) et
abîmés octet par octet, ou sur des éléments faits de valeurs (`Val`) : aucune donnée du propriétaire.

**D'abord la découpe** (`057a0dba`, adaptée de `6119188b`) : `pst.rs` (786 lignes) garde le parcours
des dossiers ; la lecture d'un élément va dans `pst/item.rs`, sa reconstruction dans
`pst/rebuild.rs`, l'écriture dans `pst/out.rs`, les valeurs dans `pst/props.rs` (`Val`, que lit
aussi `pst_pim.rs`). Pur déplacement, les parties du fork comprises (tâches, photo des contacts,
`catch_unwind` par dossier, plafond `MAX_VALUE`). Puis `Out::new` (`8ee66721`, propre au fork) : les
champs d'`Out` n'étaient plus à répéter dans chaque test.

| Commit de l'origine | Sonde | avant | après |
|---|---|---|---|
| `7f879730` (`9ddc2712`, adapté) | ses deux tests | corps seulement en RTF : **aucun HTML** ; texte brut sans le HTML que son RTF enveloppait | HTML repris, images `cid:` dans le corps |
| `8c21ad43` (`168ae198`, adapté) | sonde : HTML en Windows-1251 | « Ïðèâåò » | « Привет » ; pages de l'élément, BOM, UTF-8 mal étiqueté ; ISO-2022-KR compté |
| `d4902c12` (`82b25e6d`, repris) | son test sans l'importance | verbe 104 et icône 262 **non transférés** ; `<a><b>`, `<a>,<b>`, `a,b` lus en un seul identifiant | transféré, importance, identifiants séparés |
| `c92e285f` (`c5caf860`, repris) | sonde | aucun en-tête de fil ; **un `Message-ID` piégé ajoutait une ligne `Bcc:`** | `Thread-Index`, `Thread-Topic`, `Sensitivity` ; identifiant nettoyé ; cible de fuzz `pst_item` |
| `f186abb0` (`343a0026`, repris) | sonde | destinataire connu par son seul identifiant ponctuel **perdu** | adresse et nom pris de l'identifiant |
| `47aff562` (`c592989c`, adapté) | sonde sur un vrai fichier (`tests/fidelity.rs`) | catégories « Work », « To do » : **aucun mot-clé** | `work`, `to_do` |
| `1cbc081f` (`e08159b1`, repris) | son test | adresse Cci avec saut de ligne gardée ; caractère de contrôle dans `Subject:` | noms, adresses, identifiants tenus dans leur ligne |
| `ea6c516d` (`cfa83ece`, adapté) | ses lignes `0x0E1F` | RTF plus ancien que le texte : **son HTML par-dessus** | laissé ; RTF illisible compté |
| `99470285` (`a4f99e11`, adapté) | ses nouveaux cas | « Hello » d'un message étiqueté UTF-16 lu « 效汬 » ; `<meta charset=utf-8>` à cheval sur 4 Ko : charabia | `charset` lu seulement dans une balise `<meta>` |
| `d4433d85` (`d22573cc`, adapté) | son test de fichier abîmé, sans les comptes | **passait déjà** (le message entrait sans les deux pièces) | pièces illisibles et liens comptés (`FileLeftOut`) ; fichier vide repris de sa ligne |
| `fefd506a` (`733249f1`, adapté) | ses six cas | image montrée par `cid:LOGO@…` (casse) sortie en fichier : **trombone** à l'import comme à la relecture | règle commune `carries_file`, `draft_carries_files`, `HtmlRefs::cites` |
| `bc0308e4` (`948d2209`, adapté) | sonde mémoire (`rebuilt::memory_…`, ignorée) | fichier de 32 Mio : pic **3,85** fois le fichier | **1,46** (la source et un peu) |
| `a905da13` (`6ba2884a`, repris) | ses deux tests | 80 lignes `Received` : 5 742 160 octets tenus pour 2 876 948 ; aperçu perdu après 64 K de citation | tampon exact, aperçu entier |
| `ca3cb9e4`, `5d47a4b3`, `3b56c9f7` (`29393ebd`, adaptés) | sonde (`open::measure_…`, ignorée) | lecture seule : **passait déjà** ; 2 000 messages : **16 785 lectures** | 214 lectures ; pic 17,5 → 31 Mo sur ce petit fichier (cache partagé et second lecteur) |
| `478f363c` (`ee1864f1`, repris) | celle de `dfbaa156` | — | `OutAttachment::content_id` (pas pris en R11 faute d'usage) |
| `dfbaa156` (`c6f4a875`, adapté) | sonde | Content-ID d'un fichier non montré **perdu** ; fichier caché d'un message **écarté** | gardé ; fichiers cachés repris sauf photo de contact et occurrences modifiées |
| `26bffa88` + `1d1f8b28` (`c0450bdf`, adaptés) | ses tests du lecteur | `Outer[Middle[], empty.txt]` : **fichiers des messages joints perdus** ; message joint à lui-même recopié | `Outer[Middle[Deep[deep.txt], middle.txt], empty.txt]` ; boucle écartée |
| `fb05cf0a` (`fcf55d18`, adapté) | son test | onze messages joints de 111 octets dans 300 : **tous entrés** | le texte compte dans les 128 Mio de l'élément |
| `20792f19` (`4db4e00e`, repris) | son premier test | non-remise reconstruite en `multipart/mixed` | `multipart/report`, état d'abord, en 7bit |
| `73c19c52` (`76e9a8ed`, repris) | ses tests | signé ou chiffré : courrier simple avec un `smime.p7m` | signé tel qu'envoyé, CMS en `application/pkcs7-mime` |
| Restes de R16 et R18 (`2ad431e3`, propre au fork) | sondes sur vrais fichiers (`tests/fidelity.rs`) | terminé dans Outlook : **aucune marque** ; répondu : **sans heure** | `$FollowUpComplete` et `Flags::completed` ; `Flags::acted` à côté de 102, 103, 104 |

- **Repris tels quels** (au nom de leur auteur) : `d4902c12`, `c92e285f`, `f186abb0`, `1cbc081f`,
  `a905da13`, `478f363c`, `20792f19` (avec son `usable_headers`, venu de `b320eae8`), `73c19c52`.
- **Adaptés** (au nom du propriétaire), toujours pour la même raison : l'origine a un rapport de
  pertes par raison (`SkipReason`, `skips`), des options d'import et des statistiques de durée, que
  l'import du fork n'a pas. Ici : `Imported::charset_unsupported`, `body_not_converted`,
  `files_left_out` (`FileLeftOut` : lien, trop gros, illisible, message joint illisible, trop
  profond, au-delà de la limite) et `reading` (octets lus, lectures, réouvertures), tous écrits au
  journal ; le moteur ne lit toujours que `messages` et `skipped`. Les catégories passent par
  `keyword_for` et `is_keyword` comme celles d'Exchange (« $Junk » devient `cat_$junk`) ; la photo
  d'un contact est toujours prise parmi ses pièces (`Held::photo`) ; la règle du trombone est
  celle du fork (vérification S/MIME d'origine, `winmail.dat` dans son budget) ; le lecteur à
  cache se règle par `import_recycled` dans les tests, pas par des options.
- **Propre au fork** : `Out::new` ; les restes de R16 et R18 (`0x1090` = 1 et `0x1091`, `0x1082`
  lus comme EWS les lit : avant 1970 ou « aucune » de MAPI en 4501, pas d'heure ; pas d'heure à
  côté d'un autre verbe) ; `tests/fidelity.rs` et `tests/seeds.rs` (les graines de `pst`,
  `pst_item` et `pst_props` relues comme leurs cibles les lisent, puisque `cargo-fuzz` manque ici).
- **Découpes** : les tests passés au-delà de 800 lignes vont dans `tests/code_pages.rs`,
  `tests/headers.rs` et `tests/reader/attached.rs` (`fa4494e1`, pur déplacement) ; les trois
  cibles de fuzz des fichiers de données dans `fuzz/fuzz_targets/pst/` (`91bfc59c` : le dossier
  dépassait 16 fichiers ; les binaires gardent leurs noms). `pst.rs` : 466 lignes.
- **Ce qui dépendait de l'import suivi de l'origine** : les commits pris vivent chez l'origine dans
  son pipeline (minuterie par phase, compression avant l'écrivain, plan, reprise). Seule leur
  correction de fidélité est prise, branchée sur `Out::add_message` du fork : `bc0308e4` sans
  `tally.time` ni `Store::prepare` ; `ca3cb9e4`… sans `ImportOptions` (`chunk_kb`, `recycle`,
  `handle_cache_bytes`) ni `ImportStats`, remplacés par `Reading` et `import_recycled`.
- **Ce que verra le propriétaire** en important un `.pst` : le courrier venu d'Outlook arrive avec
  ce qu'Outlook gardait et que l'import perdait : les corps seulement en RTF et leurs images, le
  cyrillique, le grec, le chinois ou le japonais lisibles, les catégories, le drapeau terminé avec
  son jour (« Terminé le … ») et la flèche répondu ou transféré avec son heure (« Répondu le … »),
  l'importance, la sensibilité, le fil (`Thread-Index`), les fichiers des messages joints (jusqu'à
  trois niveaux), les fichiers cachés d'un message, les non-remises en rapports, le courrier signé
  ou chiffré tel qu'envoyé (signature vérifiable, déchiffrable avec sa clé). Le trombone suit ce
  que montre le message. Sur un gros fichier, beaucoup moins de lectures du disque, le fichier
  ouvert en lecture seule, et la mémoire du lecteur bornée par la réouverture. Rien ne change pour
  un fichier déjà importé : il faut le réimporter.
- **Pour P2b** (rendez-vous, contacts, demandes de réunion, groupes, journal) :
  - le lecteur d'un élément est `item_with_files(&Held { … })` (`pst.rs`) : les pièces passent
    par `Holder` (`pst/embedded.rs`) ; la photo d'un contact est prise là (`Held::photo`) ;
  - `item_of(p, recipients, files, named)` lit tout par `Val` ; `pst_pim::extract` prend la même
    fermeture ; `Named` connaît maintenant un nom par chaîne (`keywords`) ;
  - les raisons d'écartement sont `FileLeftOut` (pas de `SkipReason`) ; ajouter une raison là ;
  - `hidden(p, class)` laisse déjà de côté les occurrences modifiées d'une série
    (`afException`, ou message joint caché d'une réunion ou d'un rendez-vous) ;
  - les tests créent `Out::new(&store, "d1", &mut progress)` ; `tests/` a 14 fichiers et le
    sous-dossier `reader/` : prévoir des sous-dossiers par rôle au-delà de 16 ;
  - `docs/gaps.md` n'a pas été suivi (le fork tient ce journal-ci).
- **Vérifié** : chaque sonde avant (échec, ou déjà juste pour les deux notées) et après ; les
  suites de `quatrelle-net` (import, export, EWS), `quatrelle-mime` et `quatrelle-engine` ; les
  cibles de fuzz par `cargo check --bins` dans `fuzz/` et leurs graines par `tests/seeds.rs` ;
  `cargo xtask ci` (wasm compris).
- **Reste, remarqué** :
  - la réouverture du lecteur ne s'est jamais déclenchée sur les fichiers fabriqués (20 000
    messages, 91 Mo : 1 967 lectures, 18 Mo de pic, avec ou sans) : leurs arbres sont trop
    simples ; son effet se voit sur un vrai fichier (le test ignoré `real_files`, avec
    `QUATRELLE_PST`, dit maintenant les lectures et les réouvertures) ;
  - le lecteur des dossiers (celui qui parcourt les tables) n'est jamais rouvert : ses pages
    croissent avec le nombre de dossiers et de lignes, comme chez l'origine ;
  - sur un petit fichier, le pic de mémoire monte (17,5 → 31 Mo pour 2 000 messages) : le cache
    partagé (4 Mio au plus) et le second lecteur ;
  - les comptes nouveaux (`files_left_out`, `charset_unsupported`, `body_not_converted`) ne vont
    qu'au journal : la fenêtre ne montre toujours que les éléments écartés (`skipped`) ;
  - le partage en lecture seule sous Windows (`FILE_SHARE_READ`) n'a été essayé que par son test
    `#[cfg(windows)]`, pas lancé ici.

## 7. La rédaction des messages, rapprochée d'Outlook (2026-10-22)

Quatre chantiers, chacun dans son commit, à la suite d'un audit de l'éditeur HTML.

**Les signatures gardent leur mise en forme.** La signature était relue depuis son HTML à chaque
message (`signature_html`), ce qui perdait les tailles, les polices, le soulignement et le
centrage. Elle est désormais enregistrée telle que l'éditeur la tient (`AccountConfig::signature_styles`,
les `rich::Styles` en JSON), et l'HTML n'est relu que pour les signatures d'avant — en gardant
cette fois les polices (`fonts::mark`/`apply`), et en appliquant `inline_css` avant `clean_html`
aux signatures écrites en HTML (les styles par classe d'Outlook passaient à la trappe). La ligne
`-- ` qu'ajoutait Quatrelle avant chaque signature est devenue une option, désactivée par défaut
(Outlook n'en met pas) : Options › « Placer une ligne « -- » avant la signature »
(`Prefs::signature_dashes`, commande `view.signatureDashes`).

**Le correcteur orthographique.** Le message entier était vérifié plusieurs fois par image, et les
suggestions recalculées à chaque image tant que le menu restait ouvert. Désormais (`spellcheck.rs`) :
- la vérification se fait paragraphe par paragraphe, mémorisée par le texte du paragraphe ;
  les suggestions sont calculées une fois, à l'ouverture du menu ou du dialogue ;
- ni la signature ni l'original cité d'une réponse ne sont vérifiés ; un mot dans une écriture
  sans dictionnaire (chinois, arabe…) n'est pas souligné ;
- les suggestions FR/EN sont fusionnées et classées par distance d'édition, la langue active
  d'abord (`Spell::suggest_in`) ; elles sont en tête du menu contextuel, avec « Ignorer partout »
  (pour le message) et « Ajouter au dictionnaire » ;
- la langue se choisit par message (Révision › Langue : chaque dictionnaire, ou toutes les
  langues ; par défaut celle de l'interface), commande `compose.language` ; les dictionnaires
  Hunspell du système (`/usr/share/hunspell`, `/usr/share/myspell` : `de_DE`, `es_ES`…) sont lus
  au démarrage, jamais embarqués ;
- un mot personnel commençant par une majuscule accentuée (« Été ») est reconnu (comparaison en
  minuscules Unicode, non ASCII).

**Le ruban du message** (`rich_edit/ribbon.rs`) remplace la rangée de boutons et la seconde barre
d'outils : les onglets Message | Insertion | Options | Format du texte | Révision, avec les
widgets du ruban principal (`Groups`, `big_button`, `big_more`, `stack`). Enregistrer, Détacher
et Abandonner sont de petits boutons à droite des onglets ; le grand bouton Envoyer reste à gauche
de De/À/Cc/Objet. Nouveaux boutons : Vérifier les noms (commande `compose.checkNames`, qui
complète les adresses depuis les contacts), Ligne horizontale, choix de la police, Coller › Ne
garder que le texte. Les nouvelles icônes sont des SVG Fluent inchangés (`assets/icons/fluent/`,
une ligne chacune dans `ATTRIBUTION.md`).

**La fidélité de l'éditeur.** Les polices choisies (Arial, Calibri…) sont lues sur le système
(`Services::typeface`, fontconfig sous Linux, avec la police de substitution — Liberation Sans
pour Arial) et dessinées dans l'éditeur (`look/typefaces.rs`) ; les suites d'espaces et les
tabulations partent en `&nbsp;` (elles s'effondraient chez le destinataire) ; les paragraphes
centrés ou alignés à droite le sont sur toutes leurs lignes ; ⌘⇧Z refait sur Mac au lieu
d'effacer la mise en forme ; Couper et Copier mettent aussi l'HTML dans le presse-papiers
(`set_clipboard_html_writer`, arboard) ; le HTML collé garde ses images incorporées (`data:`)
et celles que le message possède déjà (`cid:`), en laissant les autres.

## 8. Le volet de lecture rapproché d'Outlook (2026-11-04, pour la 1.2)

Demandé par le propriétaire en regardant Outlook à côté de Quatrelle ; propre à ce fork.

- **L'en-tête** (`reading/header.rs`) : Répondre | Répondre à tous | Transférer forment un seul
  groupe (`widgets::button_group`), suivi d'Assistant (Résumer, Proposer une réponse, Classer dans
  son menu ; icône Fluent `sparkle`) et de ⋯ (Imprimer, Enregistrer en PDF, Enregistrer comme
  e-mail, Répondre par une réunion, Transférer en pièce jointe, En-têtes du message). Dans un volet
  assez large, ils sont à droite de l'expéditeur, la date dessous ; sinon sur une ligne sous
  l'en-tête, hors de son défilement, en icônes seules si même cette ligne est trop étroite. Les
  boutons du volet de lecture sont dans `look/widgets/pills.rs`.
- **Le menu clic droit d'un courrier** (`panes/list.rs`, `panes/organize.rs`) : une icône devant
  chaque commande (`widgets::menu_icon`, un atom de dessin d'egui), deux icônes Fluent ajoutées
  (`mail_dismiss` pour Ignorer, `mail_settings` pour Créer une règle).
- **Reformuler** : à côté d'Abandonner sur une réponse écrite dans le volet, dans l'onglet Message
  d'une fenêtre, et toujours dans Révision ; grisé tant que rien n'est tapé au-dessus de la
  signature et du message cité (`Session::has_written_text`). Corrigé en route : la fin du texte
  tapé cherchait la ligne `From:` en anglais seulement ; en français (« De : »), une réponse sans
  signature envoyait aussi le message cité au modèle.
- **La ligne entre les échanges** : la bordure du haut qu'Outlook met au-dessus de chaque message
  cité, et les `<hr>`, sont repérées avant la conversion du HTML (`html/rules.rs`) et dessinées en
  travers du message (`Span::rule`).
