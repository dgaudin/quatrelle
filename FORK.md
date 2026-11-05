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
