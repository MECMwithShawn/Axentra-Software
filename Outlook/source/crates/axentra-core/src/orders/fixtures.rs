//! Order emails for the tests: one per case, all from fictional `*.example`
//! shops and people. Received on Monday 2026-10-19 at noon, local time.

use std::collections::HashSet;

use chrono::{DateTime, Local, TimeZone, Utc};

use super::{Extraction, Input, extract};
use crate::junk::analyze::Severity;
use crate::model::{EmailAddress, Message};

pub(crate) struct Mail {
    pub message: Message,
    pub html: Option<String>,
    pub text: String,
}

/// Monday 2026-10-19, `hour`:00 local time.
pub(crate) fn monday(hour: u32) -> DateTime<Utc> {
    Local.with_ymd_and_hms(2026, 10, 19, hour, 0, 0).earliest().unwrap_or_default().with_timezone(&Utc)
}

impl Mail {
    pub(crate) fn new(name: &str, address: &str, subject: &str) -> Self {
        let message = Message { from: EmailAddress::new(name, address), subject: subject.into(), received_at: monday(12), ..Default::default() };
        Self { message, html: None, text: String::new() }
    }

    pub(crate) fn html(mut self, html: &str) -> Self {
        self.text = crate::html::html_to_text(html);
        self.html = Some(html.into());
        self
    }

    pub(crate) fn text(mut self, text: &str) -> Self {
        self.text = text.into();
        self
    }
}

/// What else is known about a message (all off by default).
#[derive(Default)]
pub(crate) struct Known {
    pub junk: Option<Severity>,
    pub impersonation: bool,
    pub reported: bool,
    pub allowed: bool,
    pub authenticated: bool,
    pub merchant: bool,
    pub trusted: bool,
    pub ignored: bool,
    pub own: Vec<String>,
    pub contacts: HashSet<String>,
}

pub(crate) fn run(mail: &Mail, known: &Known) -> Extraction {
    extract(&Input {
        message: &mail.message,
        html: mail.html.as_deref(),
        text: &mail.text,
        junk: known.junk,
        impersonation: known.impersonation,
        reported_phishing: known.reported,
        sender_allowed: known.allowed,
        sender_authenticated: known.authenticated,
        own_addresses: &known.own,
        contacts: &known.contacts,
        known_merchant: known.merchant,
        trusted_merchant: known.trusted,
        ignored: known.ignored,
    })
}

const BRIGHTLEAF: (&str, &str) = ("Brightleaf Home", "orders@brightleaf.example");

/// An order confirmation with an items table (cells glued by `td`, prices
/// as `&#36;`).
pub(crate) fn confirmation() -> Mail {
    Mail::new(BRIGHTLEAF.0, BRIGHTLEAF.1, "Order confirmed: #BL-48213").html(
        "<p>Thanks for shopping with Brightleaf.</p><p>Order number: BL-48213</p>\
         <table><tr><th>Item</th><th>Qty</th><th>Price</th></tr>\
         <tr><td>Linen throw blanket</td><td>1</td><td>&#36;48.00</td></tr>\
         <tr><td>Stoneware mug set</td><td>1</td><td>&#36;32.00</td></tr>\
         <tr><td>Total</td><td></td><td>&#36;80.00</td></tr></table>",
    )
}

/// Shipped with UPS, the tracking number in a table cell.
pub(crate) fn shipped_ups() -> Mail {
    Mail::new(BRIGHTLEAF.0, BRIGHTLEAF.1, "Your Brightleaf order #BL-48213 has shipped").html(
        "<table><tr><td>Shipped with UPS</td><td>Tracking number:</td><td>1Z4X7Q203660123459</td></tr></table>\
         <p>Estimated delivery: Tuesday, October 20</p>",
    )
}

pub(crate) fn out_for_delivery() -> Mail {
    Mail::new(BRIGHTLEAF.0, BRIGHTLEAF.1, "Out for delivery: your order #BL-48213")
        .text("Your package is out for delivery today. UPS tracking number 1Z4X7Q203660123459")
}

pub(crate) fn delivered() -> Mail {
    Mail::new("Northpeak Outfitters", "shop@northpeak.example", "Delivered: your Northpeak order #NP-20931")
        .text("Your package was delivered. FedEx tracking number 612900344789")
}

pub(crate) fn not_delivered() -> Mail {
    Mail::new("Northpeak Outfitters", "shop@northpeak.example", "An update on order #NP-20931")
        .text("Your package has not been delivered. We'll try again tomorrow.\nFedEx tracking number 612900344789")
}

pub(crate) fn delayed() -> Mail {
    Mail::new("Pixel & Parcel Books", "orders@pixelparcel.example", "Delayed: order #PP-77310 has a new delivery date")
        .text("Sorry! USPS tracking number 9200190301123456789017. New delivery date: Friday, October 23")
}

pub(crate) fn cancelled() -> Mail {
    Mail::new("Harbor Lane Kitchen", "orders@harborlane.example", "Your order #HL-3304 has been cancelled").text("As requested, we cancelled your order.")
}

pub(crate) fn refund() -> Mail {
    Mail::new("Harbor Lane Kitchen", "orders@harborlane.example", "Refund issued for order #HL-3304").html("<p>Refund of &#36;27.50 issued to your card.</p>")
}

/// A promotion that says "order now".
pub(crate) fn promo() -> Mail {
    Mail::new("Northpeak Outfitters", "news@northpeak.example", "30% off new arrivals – order now, ships free").html(
        "<h2>New arrivals</h2><p>Order now and save.</p>\
         <p><a href=\"https://northpeak.example/new\">Shop</a> · <a href=\"https://northpeak.example/unsubscribe\">Unsubscribe</a></p>",
    )
}

/// A contact's reply that mentions an order.
pub(crate) fn contact_chat() -> Mail {
    let mut m = Mail::new("Dana Whitfield", "dana@home.example", "Re: I ordered the cake for Friday").text("I placed order #12 with the bakery, pick up at 5");
    m.message.in_reply_to = Some("abc@home.example".into());
    m
}

/// Japanese, with a full-width order number.
pub(crate) fn japanese() -> Mail {
    Mail::new("さくら雑貨", "shop@sakura-zakka.example", "【さくら雑貨】ご注文の商品を発送しました（注文番号：ＳＺ－１００２４５）")
        .text("ご注文ありがとうございます。\n追跡番号：EJ517382041JP\nお届け予定日：2026年10月22日")
}

pub(crate) fn german() -> Mail {
    Mail::new("Bergwerk Laden", "bestellung@bergwerk.example", "Ihre Bestellung wurde versandt")
        .text("Hallo,\nBestellnummer: BW-20931-77\nSendungsnummer: JJD000390007788663366 (DHL)")
}

/// A subject full of 'İ', which changes length when lowercased.
pub(crate) fn turkish() -> Mail {
    Mail::new("Pazar", "orders@pazar.example", "İİİ SİPARİŞİNİZ KARGODA İİİ order #TR-55821 İİİ").text(&format!("{} Order number: TR-55821, shipped.", "İ".repeat(1000)))
}

/// Claims to be UPS from somewhere else, and asks for a fee.
pub(crate) fn carrier_impersonation() -> Mail {
    Mail::new("UPS", "alerts@ups-redelivery.example", "Your package is on hold").text("A $1.99 fee is due before we can deliver. Pay at https://ups-redelivery.example/pay")
}

/// An unknown sender's parcel "on hold" for a customs fee.
pub(crate) fn fee_lure() -> Mail {
    Mail::new("Parcel Desk", "delivery@parcel-redelivery.example", "Your parcel is on hold").html(
        "<p>We couldn't deliver your package: a customs fee is unpaid.</p>\
         <p>Pay a small fee to reschedule: <a href=\"https://parcel-redelivery.example/pay\">Pay now</a></p>",
    )
}

/// Tracking only, no order number.
pub(crate) fn tracking_only() -> Mail {
    Mail::new("Brightleaf Home", "tracking@brightleaf.example", "Your package is on its way").text("Tracking number: 1Z999AA10123456784")
}
