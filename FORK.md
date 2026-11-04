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
