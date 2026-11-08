// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The twenty messages "Try it on examples" puts to the language model: ten scams and ten ordinary
//! messages, half of each in French and half in English. Written for Quatrelle, none taken from
//! real mail; every address is under `example`.

/// One message to judge, and what the right answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    pub subject: &'static str,
    pub from_name: &'static str,
    pub from: &'static str,
    pub text: &'static str,
    /// Whether it is a scam.
    pub scam: bool,
    /// Whether it is written in French.
    pub french: bool,
}

const fn ex(subject: &'static str, from_name: &'static str, from: &'static str, text: &'static str, scam: bool, french: bool) -> Example {
    Example { subject, from_name, from, text, scam, french }
}

/// The examples, scams first.
pub const EXAMPLES: [Example; 20] = [
    ex(
        "Facture 4471 : nouvelles coordonnées bancaires",
        "Service comptabilité",
        "compta@fournisseur-facturation.example",
        "Bonjour, notre banque a changé. Merci de régler la facture 4471 de 18 450 euros avant vendredi sur le nouveau compte indiqué dans la pièce jointe. Ne rappelez pas l'ancien numéro, il n'est plus en service. Cordialement.",
        true,
        true,
    ),
    ex(
        "Votre mot de passe expire aujourd'hui",
        "Assistance messagerie",
        "assistance@messagerie-verif.example",
        "Votre mot de passe expire dans 2 heures. Pour garder l'accès à votre boîte, confirmez votre identifiant et votre mot de passe sur http://connexion-messagerie.example/verifier. Sans action de votre part, votre compte sera supprimé.",
        true,
        true,
    ),
    ex(
        "Colis en attente : frais de livraison de 1,99 euro",
        "Livraison Express",
        "suivi@colis-express-info.example",
        "Votre colis n'a pas pu être livré. Réglez 1,99 euro de frais de réexpédition sur http://colis-express-info.example/payer avant 24 heures, faute de quoi il sera retourné à l'expéditeur.",
        true,
        true,
    ),
    ex(
        "Urgent, restez discrète",
        "Claire Martin (directrice)",
        "claire.martin.dir@mailbox-direct.example",
        "Bonjour, je suis en réunion et je ne peux pas téléphoner. J'ai besoin de toi pour une opération confidentielle. Réponds-moi tout de suite, ne dis rien à personne pour l'instant.",
        true,
        true,
    ),
    ex(
        "Remboursement d'impôt de 412 euros",
        "Direction des finances",
        "remboursement@impots-gouv-service.example",
        "Après le calcul de votre dernière déclaration, un remboursement de 412 euros vous est dû. Pour le recevoir, saisissez vos coordonnées bancaires et le code reçu par SMS sur notre formulaire sécurisé dans les 48 heures.",
        true,
        true,
    ),
    ex(
        "Payment overdue: update your bank details",
        "Accounts Payable",
        "billing@supplier-payments.example",
        "Dear customer, invoice 88213 for 12,900 USD is overdue. Our bank details have changed: please send the transfer today to the new account in the attached form. Reply only to this address, the old line is closed.",
        true,
        false,
    ),
    ex(
        "Your account has been locked",
        "Security Team",
        "security@account-protect.example",
        "We noticed unusual activity. Your mailbox is locked until you sign in again and confirm your password at http://account-protect.example/unlock. If you do nothing within 24 hours the account is closed for good.",
        true,
        false,
    ),
    ex(
        "Quick favour, gift cards",
        "Daniel Reed (CEO)",
        "daniel.reed.ceo@outlook-mail.example",
        "Are you at your desk? I'm in a meeting and need five 100 dollar gift cards for a client today. Buy them and send me the codes by reply, I'll pay you back this evening. Keep this between us.",
        true,
        false,
    ),
    ex(
        "Congratulations, you won a prize",
        "Prize Desk",
        "winner@big-prize-draw.example",
        "You have been picked as the winner of 50,000 dollars in our yearly draw. To release the money, pay a 95 dollar processing fee and send us your passport number and bank account today.",
        true,
        false,
    ),
    ex(
        "Document to sign: contract of employment",
        "E-Sign Service",
        "docs@sign-your-docs.example",
        "A document is waiting for your signature. Open the attached page, then log in with your email password to view and sign it. The link expires today.",
        true,
        false,
    ),
    ex(
        "Question sur la livraison de mardi",
        "Hélène Roux",
        "helene.roux@client-atelier.example",
        "Bonjour, pouvez-vous me confirmer que la commande de mardi arrivera bien à l'atelier avant midi ? Nous avons du monde qui l'attend. Merci d'avance et bonne journée.",
        false,
        true,
    ),
    ex(
        "Changement d'horaire de la réunion de jeudi",
        "Paul Girard",
        "paul.girard@partenaire-studio.example",
        "Bonjour à tous, la réunion de jeudi est décalée à 15 h en salle Verne, au lieu de 14 h. L'ordre du jour ne change pas. À jeudi.",
        false,
        true,
    ),
    ex(
        "La lettre d'information de novembre",
        "Association des jardins",
        "lettre@jardins-partages.example",
        "Au programme ce mois-ci : la journée de plantation du 14, l'atelier compost et le calendrier des permanences. Vous recevez ce message car vous êtes inscrit à notre lettre ; vous pouvez vous désinscrire à tout moment.",
        false,
        true,
    ),
    ex(
        "Votre commande est expédiée",
        "Librairie du Port",
        "commandes@librairie-du-port.example",
        "Bonjour, votre commande 20431 a été expédiée aujourd'hui et arrivera sous trois à cinq jours. Vous trouverez le suivi dans votre compte client sur notre site. Merci de votre confiance.",
        false,
        true,
    ),
    ex(
        "Facture de novembre de votre fournisseur habituel",
        "Marc Lefèvre",
        "marc.lefevre@papeterie-lefevre.example",
        "Bonjour, ci-joint la facture de novembre, aux mêmes conditions que d'habitude et avec les mêmes coordonnées que les mois précédents. N'hésitez pas à m'appeler pour la moindre question.",
        false,
        true,
    ),
    ex(
        "A question about next week's order",
        "Alex Turner",
        "alex.turner@customer-works.example",
        "Hi, could you tell me whether the 40 chairs we ordered will be ready by the 12th? We're planning the move that week. Thanks, Alex.",
        false,
        false,
    ),
    ex(
        "Meeting moved to Thursday",
        "Priya Nair",
        "priya.nair@partner-studio.example",
        "Hi all, our review moves to Thursday at 3 pm in the small room because of a clash. The agenda stays the same. See you then.",
        false,
        false,
    ),
    ex(
        "The November newsletter",
        "Riverside Cycling Club",
        "news@riverside-cycling.example",
        "This month: the autumn ride on the 21st, new members' evening and the winter route map. You receive this message because you joined our mailing list; you can unsubscribe at any time.",
        false,
        false,
    ),
    ex(
        "Your order has shipped",
        "Harbour Books",
        "orders@harbour-books.example",
        "Hello, your order 55102 shipped today and should arrive within three to five working days. You can follow it from your account on our website. Thank you for shopping with us.",
        false,
        false,
    ),
    ex(
        "Invoice for October",
        "Nora Fields",
        "nora.fields@print-supplies.example",
        "Hello, the October invoice is attached, on the usual terms and with the same payment details as before. Please call me if anything looks off. Best wishes, Nora.",
        false,
        false,
    ),
];
