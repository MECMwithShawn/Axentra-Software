//! Smart categories: every entry is placed in a category (Banking, Credit cards, Travel, ...)
//! without moving it between groups. Priority: a category the user chose (stored in the entry's
//! custom data, so KeePass preserves it), then a known website, then title keywords, then the
//! names of the groups it sits in, then the website host as a last hint.

use keepass::db::{CustomDataItem, CustomDataValue, Entry, EntryRef, Times};

use crate::search::host_of;

pub const CATEGORY_KEY: &str = "AxentraVault.Category";
/// Stored as "category:<key>". The colon keeps the value out of the base64 alphabet: the
/// `keepass` crate reads custom-data text that happens to be valid base64 (e.g. "shopping")
/// back as binary, while KeePass itself always treats it as text.
const VALUE_PREFIX: &str = "category:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    Financial,
    Personal,
    Digital,
    Work,
    Other,
}

impl Area {
    pub const ALL: [Area; 5] = [
        Area::Financial,
        Area::Personal,
        Area::Digital,
        Area::Work,
        Area::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Area::Financial => "Financial",
            Area::Personal => "Personal",
            Area::Digital => "Digital",
            Area::Work => "Work",
            Area::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Banking,
    CreditCards,
    CreditBureaus,
    Investing,
    Bills,
    Insurance,
    Loans,
    Email,
    Shopping,
    Travel,
    Health,
    Government,
    Education,
    Home,
    Auto,
    Developer,
    Software,
    Gaming,
    Social,
    Streaming,
    Devices,
    Jobs,
    Uncategorized,
}

impl Category {
    pub const ALL: [Category; 23] = [
        Category::Banking,
        Category::CreditCards,
        Category::CreditBureaus,
        Category::Investing,
        Category::Bills,
        Category::Insurance,
        Category::Loans,
        Category::Email,
        Category::Shopping,
        Category::Travel,
        Category::Health,
        Category::Government,
        Category::Education,
        Category::Home,
        Category::Auto,
        Category::Developer,
        Category::Software,
        Category::Gaming,
        Category::Social,
        Category::Streaming,
        Category::Devices,
        Category::Jobs,
        Category::Uncategorized,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Banking => "Banking",
            Category::CreditCards => "Credit cards",
            Category::CreditBureaus => "Credit & bureaus",
            Category::Investing => "Investing & crypto",
            Category::Bills => "Bills & utilities",
            Category::Insurance => "Insurance",
            Category::Loans => "Loans & mortgage",
            Category::Email => "Email & accounts",
            Category::Shopping => "Shopping",
            Category::Travel => "Travel",
            Category::Health => "Health",
            Category::Government => "Government & ID",
            Category::Education => "Education",
            Category::Home => "Home & rent",
            Category::Auto => "Auto",
            Category::Developer => "Developer & cloud",
            Category::Software => "Software & licenses",
            Category::Gaming => "Gaming",
            Category::Social => "Social & media",
            Category::Streaming => "Streaming",
            Category::Devices => "Devices & network",
            Category::Jobs => "Jobs & employers",
            Category::Uncategorized => "Uncategorized",
        }
    }

    pub fn area(self) -> Area {
        use Category::*;
        match self {
            Banking | CreditCards | CreditBureaus | Investing | Bills | Insurance | Loans => {
                Area::Financial
            }
            Email | Shopping | Travel | Health | Government | Education | Home | Auto => {
                Area::Personal
            }
            Developer | Software | Gaming | Social | Streaming | Devices => Area::Digital,
            Jobs => Area::Work,
            Uncategorized => Area::Other,
        }
    }

    /// Stable identifier stored in custom data (independent of the display label).
    pub fn key(self) -> &'static str {
        use Category::*;
        match self {
            Banking => "banking",
            CreditCards => "credit-cards",
            CreditBureaus => "credit-bureaus",
            Investing => "investing",
            Bills => "bills",
            Insurance => "insurance",
            Loans => "loans",
            Email => "email",
            Shopping => "shopping",
            Travel => "travel",
            Health => "health",
            Government => "government",
            Education => "education",
            Home => "home",
            Auto => "auto",
            Developer => "developer",
            Software => "software",
            Gaming => "gaming",
            Social => "social",
            Streaming => "streaming",
            Devices => "devices",
            Jobs => "jobs",
            Uncategorized => "uncategorized",
        }
    }

    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.key() == key)
    }
}

/// Where a category came from, so the UI can say "set by you" vs "automatic".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Manual,
    Website,
    Title,
    Group,
    None,
}

use Category::*;

/// Registrable-domain suffixes (matched against the end of the host).
const DOMAINS: &[(&str, Category)] = &[
    // Banking
    ("chase.com", Banking),
    ("bankofamerica.com", Banking),
    ("wellsfargo.com", Banking),
    ("citi.com", Banking),
    ("usbank.com", Banking),
    ("pnc.com", Banking),
    ("truist.com", Banking),
    ("tdbank.com", Banking),
    ("capitalone.com", Banking),
    ("ally.com", Banking),
    ("sofi.com", Banking),
    ("chime.com", Banking),
    ("navyfederal.org", Banking),
    ("usaa.com", Banking),
    ("penfed.org", Banking),
    ("schwab.com", Banking),
    ("regions.com", Banking),
    ("fifththirdbank.com", Banking),
    ("huntington.com", Banking),
    ("citizensbank.com", Banking),
    ("marcus.com", Banking),
    ("discover.com", CreditCards),
    ("paypal.com", Banking),
    ("venmo.com", Banking),
    ("zellepay.com", Banking),
    ("cash.app", Banking),
    ("wise.com", Banking),
    ("revolut.com", Banking),
    ("varomoney.com", Banking),
    ("synchronybank.com", Banking),
    ("americanexpress.com", CreditCards),
    // Credit cards
    ("amex.com", CreditCards),
    ("synchrony.com", CreditCards),
    ("mysynchrony.com", CreditCards),
    ("barclaycardus.com", CreditCards),
    ("creditonebank.com", CreditCards),
    ("mercury.com", CreditCards),
    ("apple.com/apple-card", CreditCards),
    ("card.apple.com", CreditCards),
    ("affirm.com", Loans),
    ("klarna.com", Loans),
    ("afterpay.com", Loans),
    ("milestonecard.com", CreditCards),
    ("mission.lane", CreditCards),
    ("missionlane.com", CreditCards),
    ("petalcard.com", CreditCards),
    ("brex.com", CreditCards),
    ("comenity.net", CreditCards),
    ("target.com/redcard", CreditCards),
    // Credit and bureaus
    ("experian.com", CreditBureaus),
    ("equifax.com", CreditBureaus),
    ("transunion.com", CreditBureaus),
    ("creditkarma.com", CreditBureaus),
    ("annualcreditreport.com", CreditBureaus),
    ("myfico.com", CreditBureaus),
    ("creditsesame.com", CreditBureaus),
    ("lexisnexis.com", CreditBureaus),
    ("chexsystems.com", CreditBureaus),
    // Investing and crypto
    ("fidelity.com", Investing),
    ("vanguard.com", Investing),
    ("robinhood.com", Investing),
    ("etrade.com", Investing),
    ("webull.com", Investing),
    ("tdameritrade.com", Investing),
    ("interactivebrokers.com", Investing),
    ("acorns.com", Investing),
    ("stash.com", Investing),
    ("betterment.com", Investing),
    ("wealthfront.com", Investing),
    ("m1.com", Investing),
    ("coinbase.com", Investing),
    ("binance.us", Investing),
    ("binance.com", Investing),
    ("kraken.com", Investing),
    ("gemini.com", Investing),
    ("crypto.com", Investing),
    ("coinrule.com", Investing),
    ("tradingview.com", Investing),
    ("metamask.io", Investing),
    ("ledger.com", Investing),
    ("bookmap.com", Investing),
    ("databento.com", Investing),
    ("ninjatrader.com", Investing),
    ("tradestation.com", Investing),
    ("tastytrade.com", Investing),
    ("public.com", Investing),
    ("treasurydirect.gov", Investing),
    ("empower.com", Investing),
    ("principal.com", Investing),
    ("tiaa.org", Investing),
    // Bills and utilities
    ("fpl.com", Bills),
    ("duke-energy.com", Bills),
    ("xfinity.com", Bills),
    ("comcast.com", Bills),
    ("att.com", Bills),
    ("verizon.com", Bills),
    ("t-mobile.com", Bills),
    ("spectrum.net", Bills),
    ("spectrum.com", Bills),
    ("cox.com", Bills),
    ("frontier.com", Bills),
    ("centurylink.com", Bills),
    ("teco.com", Bills),
    ("tecoenergy.com", Bills),
    ("pge.com", Bills),
    ("coned.com", Bills),
    ("nationalgrid.com", Bills),
    ("dominionenergy.com", Bills),
    ("southerncompany.com", Bills),
    ("starlink.com", Bills),
    ("mintmobile.com", Bills),
    ("visible.com", Bills),
    ("cricketwireless.com", Bills),
    ("boostmobile.com", Bills),
    ("googlefi.com", Bills),
    ("fi.google.com", Bills),
    ("waste management", Bills),
    ("wm.com", Bills),
    // Insurance
    ("geico.com", Insurance),
    ("progressive.com", Insurance),
    ("statefarm.com", Insurance),
    ("allstate.com", Insurance),
    ("libertymutual.com", Insurance),
    ("nationwide.com", Insurance),
    ("farmers.com", Insurance),
    ("travelers.com", Insurance),
    ("lemonade.com", Insurance),
    ("root.com", Insurance),
    ("metlife.com", Insurance),
    ("aflac.com", Insurance),
    ("bcbs.com", Insurance),
    ("floridablue.com", Insurance),
    ("humana.com", Insurance),
    ("cigna.com", Insurance),
    ("aetna.com", Insurance),
    ("uhc.com", Insurance),
    ("healthcare.gov", Insurance),
    ("oscarhealth.com", Insurance),
    ("ambetter.com", Insurance),
    ("guardianlife.com", Insurance),
    ("primerica.com", Insurance),
    // Loans and mortgage
    ("rocketmortgage.com", Loans),
    ("mrcooper.com", Loans),
    ("quickenloans.com", Loans),
    ("navient.com", Loans),
    ("nelnet.com", Loans),
    ("studentaid.gov", Loans),
    ("mohela.com", Loans),
    ("aidvantage.com", Loans),
    ("upstart.com", Loans),
    ("lendingclub.com", Loans),
    ("prosper.com", Loans),
    ("carvana.com", Auto),
    ("santanderconsumerusa.com", Loans),
    ("ally.com/auto", Loans),
    ("toyotafinancial.com", Loans),
    ("gmfinancial.com", Loans),
    ("hondafinancialservices.com", Loans),
    ("loandepot.com", Loans),
    ("freedommortgage.com", Loans),
    // Email and accounts
    ("outlook.com", Email),
    ("outlook.live.com", Email),
    ("live.com", Email),
    ("hotmail.com", Email),
    ("login.microsoftonline.com", Email),
    ("account.microsoft.com", Email),
    ("gmail.com", Email),
    ("mail.google.com", Email),
    ("accounts.google.com", Email),
    ("myaccount.google.com", Email),
    ("yahoo.com", Email),
    ("proton.me", Email),
    ("protonmail.com", Email),
    ("icloud.com", Email),
    ("appleid.apple.com", Email),
    ("account.apple.com", Email),
    ("aol.com", Email),
    ("zoho.com", Email),
    ("fastmail.com", Email),
    ("tuta.com", Email),
    ("1password.com", Email),
    ("bitwarden.com", Email),
    ("lastpass.com", Email),
    ("authy.com", Email),
    ("okta.com", Email),
    // Shopping
    ("amazon.com", Shopping),
    ("ebay.com", Shopping),
    ("walmart.com", Shopping),
    ("target.com", Shopping),
    ("bestbuy.com", Shopping),
    ("costco.com", Shopping),
    ("samsclub.com", Shopping),
    ("homedepot.com", Home),
    ("lowes.com", Home),
    ("etsy.com", Shopping),
    ("newegg.com", Shopping),
    ("bhphotovideo.com", Shopping),
    ("aliexpress.com", Shopping),
    ("temu.com", Shopping),
    ("shein.com", Shopping),
    ("wayfair.com", Home),
    ("ikea.com", Home),
    ("chewy.com", Shopping),
    ("instacart.com", Shopping),
    ("doordash.com", Shopping),
    ("ubereats.com", Shopping),
    ("grubhub.com", Shopping),
    ("starbucks.com", Shopping),
    ("nike.com", Shopping),
    ("apple.com/shop", Shopping),
    ("store.apple.com", Shopping),
    ("shop.aquatruwater.com", Shopping),
    ("aquatruwater.com", Shopping),
    ("dell.com", Shopping),
    ("microcenter.com", Shopping),
    ("adorama.com", Shopping),
    ("macys.com", Shopping),
    ("kohls.com", Shopping),
    ("nordstrom.com", Shopping),
    ("zappos.com", Shopping),
    ("cvs.com", Health),
    ("walgreens.com", Health),
    // Travel
    ("aa.com", Travel),
    ("delta.com", Travel),
    ("united.com", Travel),
    ("southwest.com", Travel),
    ("alaskaair.com", Travel),
    ("jetblue.com", Travel),
    ("spirit.com", Travel),
    ("flyfrontier.com", Travel),
    ("allegiantair.com", Travel),
    ("airbnb.com", Travel),
    ("vrbo.com", Travel),
    ("booking.com", Travel),
    ("expedia.com", Travel),
    ("hotels.com", Travel),
    ("marriott.com", Travel),
    ("hilton.com", Travel),
    ("hyatt.com", Travel),
    ("ihg.com", Travel),
    ("wyndhamhotels.com", Travel),
    ("choicehotels.com", Travel),
    ("kayak.com", Travel),
    ("priceline.com", Travel),
    ("tripadvisor.com", Travel),
    ("uber.com", Travel),
    ("lyft.com", Travel),
    ("hertz.com", Travel),
    ("enterprise.com", Travel),
    ("avis.com", Travel),
    ("nationalcar.com", Travel),
    ("amtrak.com", Travel),
    ("tsa.gov", Travel),
    ("clear.com", Travel),
    ("carnival.com", Travel),
    ("royalcaribbean.com", Travel),
    ("ncl.com", Travel),
    // Health
    ("mychart.com", Health),
    ("mychart.org", Health),
    ("myhealthrecord.com", Health),
    ("goodrx.com", Health),
    ("zocdoc.com", Health),
    ("teladoc.com", Health),
    ("labcorp.com", Health),
    ("questdiagnostics.com", Health),
    ("hims.com", Health),
    ("hers.com", Health),
    ("23andme.com", Health),
    ("ancestry.com", Health),
    ("fitbit.com", Health),
    ("myfitnesspal.com", Health),
    ("strava.com", Health),
    ("peloton.com", Health),
    ("onepeloton.com", Health),
    ("whoop.com", Health),
    ("ouraring.com", Health),
    ("va.gov", Government),
    ("massageenvy.com", Health),
    ("elementsmassage.com", Health),
    // Government and ID
    ("irs.gov", Government),
    ("ssa.gov", Government),
    ("login.gov", Government),
    ("id.me", Government),
    ("usps.com", Government),
    ("dmv.org", Government),
    ("flhsmv.gov", Government),
    ("mydmvportal.flhsmv.gov", Government),
    ("travel.state.gov", Government),
    ("uscis.gov", Government),
    ("myfloridacounty.com", Government),
    ("floridarevenue.com", Government),
    ("myflorida.com", Government),
    ("vote.gov", Government),
    ("cbp.gov", Government),
    ("dod.mil", Government),
    ("mil", Government),
    ("milconnect.dmdc.osd.mil", Government),
    ("mypay.dfas.mil", Government),
    ("tricare.mil", Government),
    ("dfas.mil", Government),
    ("usajobs.gov", Jobs),
    // Education
    ("coursera.org", Education),
    ("udemy.com", Education),
    ("edx.org", Education),
    ("khanacademy.org", Education),
    ("pluralsight.com", Education),
    ("linkedin.com/learning", Education),
    ("duolingo.com", Education),
    ("comptia.org", Education),
    ("pearsonvue.com", Education),
    ("credly.com", Education),
    ("isc2.org", Education),
    ("cisco.com/learning", Education),
    ("certmetrics.com", Education),
    ("examcompass.com", Education),
    ("canvas.instructure.com", Education),
    ("instructure.com", Education),
    ("blackboard.com", Education),
    ("studentloans.gov", Loans),
    ("fafsa.gov", Education),
    ("collegeboard.org", Education),
    ("wgu.edu", Education),
    ("edu", Education),
    ("quizlet.com", Education),
    ("chegg.com", Education),
    ("brilliant.org", Education),
    // Home and rent
    ("apartments.com", Home),
    ("zillow.com", Home),
    ("redfin.com", Home),
    ("realtor.com", Home),
    ("rentcafe.com", Home),
    ("appfolio.com", Home),
    ("buildium.com", Home),
    ("residentportal.com", Home),
    ("paylease.com", Home),
    ("zego.io", Home),
    ("rentpayment.com", Home),
    ("yardi.com", Home),
    ("ring.com", Devices),
    ("simplisafe.com", Home),
    ("adt.com", Home),
    ("nest.com", Devices),
    // Auto
    ("jeep.com", Auto),
    ("mopar.com", Auto),
    ("ford.com", Auto),
    ("toyota.com", Auto),
    ("tesla.com", Auto),
    ("honda.com", Auto),
    ("chevrolet.com", Auto),
    ("gm.com", Auto),
    ("carfax.com", Auto),
    ("kbb.com", Auto),
    ("autotrader.com", Auto),
    ("cars.com", Auto),
    ("carmax.com", Auto),
    ("sunpass.com", Auto),
    ("ezpass.com", Auto),
    ("e-zpassny.com", Auto),
    ("gasbuddy.com", Auto),
    ("aaa.com", Auto),
    ("advanceautoparts.com", Auto),
    ("autozone.com", Auto),
    ("oreillyauto.com", Auto),
    ("jeepcommunity.com", Auto),
    ("myuconnect.com", Auto),
    ("driveuconnect.com", Auto),
    ("stellantis.com", Auto),
    // Developer and cloud
    ("github.com", Developer),
    ("gitlab.com", Developer),
    ("bitbucket.org", Developer),
    ("aws.amazon.com", Developer),
    ("console.aws.amazon.com", Developer),
    ("signin.aws.amazon.com", Developer),
    ("lightsail.aws.amazon.com", Developer),
    ("portal.azure.com", Developer),
    ("azure.com", Developer),
    ("azure.microsoft.com", Developer),
    ("cloud.google.com", Developer),
    ("console.cloud.google.com", Developer),
    ("firebase.google.com", Developer),
    ("developer.apple.com", Developer),
    ("appstoreconnect.apple.com", Developer),
    ("developer.spotify.com", Developer),
    ("developer.android.com", Developer),
    ("play.google.com/console", Developer),
    ("developers.google.com", Developer),
    ("digitalocean.com", Developer),
    ("linode.com", Developer),
    ("vultr.com", Developer),
    ("heroku.com", Developer),
    ("vercel.com", Developer),
    ("netlify.com", Developer),
    ("cloudflare.com", Developer),
    ("godaddy.com", Developer),
    ("namecheap.com", Developer),
    ("squarespace.com", Developer),
    ("wix.com", Developer),
    ("hostinger.com", Developer),
    ("openai.com", Developer),
    ("platform.openai.com", Developer),
    ("anthropic.com", Developer),
    ("console.anthropic.com", Developer),
    ("claude.ai", Developer),
    ("chatgpt.com", Developer),
    ("huggingface.co", Developer),
    ("docker.com", Developer),
    ("npmjs.com", Developer),
    ("crates.io", Developer),
    ("pypi.org", Developer),
    ("stackoverflow.com", Developer),
    ("atlassian.com", Developer),
    ("atlassian.net", Developer),
    ("jetbrains.com", Developer),
    ("postman.com", Developer),
    ("mongodb.com", Developer),
    ("supabase.com", Developer),
    ("render.com", Developer),
    ("fly.io", Developer),
    ("ngrok.com", Developer),
    ("tailscale.com", Developer),
    ("twilio.com", Developer),
    ("sendgrid.com", Developer),
    ("stripe.com", Developer),
    ("perplexity.ai", Developer),
    ("ollama.com", Developer),
    ("expo.dev", Developer),
    ("expo.io", Developer),
    ("macincloud.com", Developer),
    ("virustotal.com", Developer),
    // Software and licenses
    ("adobe.com", Software),
    ("microsoft.com", Software),
    ("office.com", Software),
    ("microsoft365.com", Software),
    ("broadcom.com", Software),
    ("support.broadcom.com", Software),
    ("vmware.com", Software),
    ("autodesk.com", Software),
    ("nitropdf.com", Software),
    ("gonitro.com", Software),
    ("macrium.com", Software),
    ("malwarebytes.com", Software),
    ("lookout.com", Software),
    ("norton.com", Software),
    ("mcafee.com", Software),
    ("bitdefender.com", Software),
    ("eset.com", Software),
    ("kaspersky.com", Software),
    ("nordvpn.com", Software),
    ("expressvpn.com", Software),
    ("protonvpn.com", Software),
    ("surfshark.com", Software),
    ("glasswire.com", Software),
    ("adlock.com", Software),
    ("dropbox.com", Software),
    ("box.com", Software),
    ("onedrive.live.com", Software),
    ("drive.google.com", Software),
    ("evernote.com", Software),
    ("notion.so", Software),
    ("canva.com", Software),
    ("grammarly.com", Software),
    ("zoom.us", Software),
    ("slack.com", Software),
    ("discord.com", Social),
    ("docusign.com", Software),
    ("turbotax.intuit.com", Software),
    ("intuit.com", Software),
    ("quickbooks.intuit.com", Software),
    ("hrblock.com", Software),
    ("wondershare.com", Software),
    ("speechify.com", Software),
    ("elevenlabs.io", Software),
    ("descript.com", Software),
    ("setapp.com", Software),
    // Gaming
    ("ea.com", Gaming),
    ("origin.com", Gaming),
    ("steampowered.com", Gaming),
    ("steamcommunity.com", Gaming),
    ("epicgames.com", Gaming),
    ("xbox.com", Gaming),
    ("playstation.com", Gaming),
    ("nintendo.com", Gaming),
    ("battle.net", Gaming),
    ("blizzard.com", Gaming),
    ("riotgames.com", Gaming),
    ("ubisoft.com", Gaming),
    ("rockstargames.com", Gaming),
    ("gog.com", Gaming),
    ("twitch.tv", Streaming),
    ("roblox.com", Gaming),
    ("minecraft.net", Gaming),
    ("bethesda.net", Gaming),
    ("activision.com", Gaming),
    ("2k.com", Gaming),
    // Social and media
    ("facebook.com", Social),
    ("instagram.com", Social),
    ("x.com", Social),
    ("twitter.com", Social),
    ("linkedin.com", Jobs),
    ("reddit.com", Social),
    ("tiktok.com", Social),
    ("snapchat.com", Social),
    ("pinterest.com", Social),
    ("threads.net", Social),
    ("bsky.app", Social),
    ("mastodon.social", Social),
    ("whatsapp.com", Social),
    ("telegram.org", Social),
    ("signal.org", Social),
    ("nextdoor.com", Social),
    ("medium.com", Social),
    ("substack.com", Social),
    ("quora.com", Social),
    ("tumblr.com", Social),
    ("youtube.com", Streaming),
    ("myspace.com", Social),
    // Streaming
    ("netflix.com", Streaming),
    ("hulu.com", Streaming),
    ("disneyplus.com", Streaming),
    ("max.com", Streaming),
    ("hbomax.com", Streaming),
    ("primevideo.com", Streaming),
    ("paramountplus.com", Streaming),
    ("peacocktv.com", Streaming),
    ("spotify.com", Streaming),
    ("music.apple.com", Streaming),
    ("tidal.com", Streaming),
    ("pandora.com", Streaming),
    ("audible.com", Streaming),
    ("siriusxm.com", Streaming),
    ("crunchyroll.com", Streaming),
    ("sling.com", Streaming),
    ("youtubetv.com", Streaming),
    ("tv.youtube.com", Streaming),
    ("fubo.tv", Streaming),
    ("plex.tv", Streaming),
    ("tv.apple.com", Streaming),
    ("deezer.com", Streaming),
    ("kick.com", Streaming),
    ("winamp.com", Streaming),
    // Devices and network
    ("asus.com", Devices),
    ("router.asus.com", Devices),
    ("netgear.com", Devices),
    ("tp-link.com", Devices),
    ("tplinkcloud.com", Devices),
    ("linksys.com", Devices),
    ("eero.com", Devices),
    ("ui.com", Devices),
    ("unifi.ui.com", Devices),
    ("synology.com", Devices),
    ("qnap.com", Devices),
    ("wyze.com", Devices),
    ("arlo.com", Devices),
    ("ecobee.com", Devices),
    ("smartthings.com", Devices),
    ("philips-hue.com", Devices),
    ("meethue.com", Devices),
    ("alexa.amazon.com", Devices),
    ("home.google.com", Devices),
    ("samsung.com", Devices),
    ("lg.com", Devices),
    ("sonos.com", Devices),
    ("roku.com", Devices),
    ("garmin.com", Devices),
    ("logitech.com", Devices),
    ("razer.com", Devices),
    ("alienware.com", Devices),
    ("nvidia.com", Devices),
    // Jobs and employers
    ("indeed.com", Jobs),
    ("glassdoor.com", Jobs),
    ("ziprecruiter.com", Jobs),
    ("monster.com", Jobs),
    ("dice.com", Jobs),
    ("clearancejobs.com", Jobs),
    ("workday.com", Jobs),
    ("myworkday.com", Jobs),
    ("myworkdayjobs.com", Jobs),
    ("taleo.net", Jobs),
    ("icims.com", Jobs),
    ("greenhouse.io", Jobs),
    ("lever.co", Jobs),
    ("adp.com", Jobs),
    ("paychex.com", Jobs),
    ("gusto.com", Jobs),
    ("paylocity.com", Jobs),
    ("ultipro.com", Jobs),
    ("ukg.com", Jobs),
    ("rtx.com", Jobs),
    ("raytheon.com", Jobs),
    ("lockheedmartin.com", Jobs),
    ("northropgrumman.com", Jobs),
    ("boeing.com", Jobs),
    ("l3harris.com", Jobs),
    ("gdit.com", Jobs),
    ("leidos.com", Jobs),
    ("saic.com", Jobs),
    ("caci.com", Jobs),
    ("boozallen.com", Jobs),
    ("upwork.com", Jobs),
    ("fiverr.com", Jobs),
    ("compassion.com", Other_),
    ("mycompassion.com", Other_),
];

/// Placeholder to keep the table aligned: maps to Uncategorized (known site, no category).
#[allow(non_upper_case_globals)]
const Other_: Category = Uncategorized;

/// Title keywords, checked as whole words or phrases (lowercase).
const KEYWORDS: &[(&str, Category)] = &[
    ("credit union", Banking),
    ("savings", Banking),
    ("checking", Banking),
    ("bank", Banking),
    ("banking", Banking),
    ("federal credit", Banking),
    ("fcu", Banking),
    ("ira", Investing),
    ("401k", Investing),
    ("401(k)", Investing),
    ("brokerage", Investing),
    ("stocks", Investing),
    ("crypto", Investing),
    ("bitcoin", Investing),
    ("wallet", Investing),
    ("trading", Investing),
    ("credit card", CreditCards),
    ("visa", CreditCards),
    ("mastercard", CreditCards),
    ("amex", CreditCards),
    ("american express", CreditCards),
    ("discover", CreditCards),
    ("card", CreditCards),
    ("credit bureau", CreditBureaus),
    ("credit report", CreditBureaus),
    ("credit score", CreditBureaus),
    ("experian", CreditBureaus),
    ("equifax", CreditBureaus),
    ("transunion", CreditBureaus),
    ("credit karma", CreditBureaus),
    ("electric", Bills),
    ("power", Bills),
    ("water", Bills),
    ("utility", Bills),
    ("utilities", Bills),
    ("internet", Bills),
    ("phone bill", Bills),
    ("wireless", Bills),
    ("cable", Bills),
    ("gas company", Bills),
    ("trash", Bills),
    ("sewer", Bills),
    ("insurance", Insurance),
    ("dental", Insurance),
    ("vision plan", Insurance),
    ("loan", Loans),
    ("mortgage", Loans),
    ("student loan", Loans),
    ("financing", Loans),
    ("lender", Loans),
    ("email", Email),
    ("e-mail", Email),
    ("gmail", Email),
    ("outlook", Email),
    ("apple id", Email),
    ("microsoft account", Email),
    ("google account", Email),
    ("icloud", Email),
    ("password manager", Email),
    ("store", Shopping),
    ("shop", Shopping),
    ("orders", Shopping),
    ("online order", Shopping),
    ("airlines", Travel),
    ("airline", Travel),
    ("airways", Travel),
    ("mileage", Travel),
    ("frequent flyer", Travel),
    ("hotel", Travel),
    ("rewards", Travel),
    ("rental car", Travel),
    ("cruise", Travel),
    ("travel", Travel),
    ("doctor", Health),
    ("medical", Health),
    ("pharmacy", Health),
    ("health", Health),
    ("clinic", Health),
    ("hospital", Health),
    ("dentist", Health),
    ("massage", Health),
    ("fitness", Health),
    ("gym", Health),
    ("supplements", Health),
    ("dmv", Government),
    ("passport", Government),
    ("license", Government),
    ("tax", Government),
    ("irs", Government),
    ("social security", Government),
    ("dod", Government),
    ("military", Government),
    ("va ", Government),
    ("court", Government),
    ("county", Government),
    ("state of", Government),
    ("school", Education),
    ("university", Education),
    ("college", Education),
    ("course", Education),
    ("certification", Education),
    ("certifications", Education),
    ("exam", Education),
    ("training", Education),
    ("comptia", Education),
    ("cissp", Education),
    ("rent", Home),
    ("apartment", Home),
    ("landlord", Home),
    ("hoa", Home),
    ("lease", Home),
    ("home", Home),
    ("jeep", Auto),
    ("car", Auto),
    ("vehicle", Auto),
    ("toll", Auto),
    ("sunpass", Auto),
    ("auto", Auto),
    ("developer", Developer),
    ("api", Developer),
    ("server", Developer),
    ("cloud", Developer),
    ("hosting", Developer),
    ("domain", Developer),
    ("ssh", Developer),
    ("aws", Developer),
    ("azure", Developer),
    ("github", Developer),
    ("text-to-speech", Developer),
    ("license key", Software),
    ("product key", Software),
    ("serial", Software),
    ("subscription", Software),
    ("software", Software),
    ("vpn", Software),
    ("antivirus", Software),
    ("paid software", Software),
    ("game", Gaming),
    ("games", Gaming),
    ("gaming", Gaming),
    ("steam", Gaming),
    ("xbox", Gaming),
    ("playstation", Gaming),
    ("nintendo", Gaming),
    ("wifi", Devices),
    ("wi-fi", Devices),
    ("router", Devices),
    ("modem", Devices),
    ("nas", Devices),
    ("camera", Devices),
    ("printer", Devices),
    ("smart home", Devices),
    ("thermostat", Devices),
    ("job", Jobs),
    ("jobs", Jobs),
    ("employer", Jobs),
    ("payroll", Jobs),
    ("timesheet", Jobs),
    ("time keeping", Jobs),
    ("timekeeping", Jobs),
    ("hr portal", Jobs),
    ("benefits", Jobs),
    ("raytheon", Jobs),
    ("rtx", Jobs),
    ("streaming", Streaming),
    ("music", Streaming),
    ("podcast", Streaming),
    ("social", Social),
    ("forum", Social),
    ("ea games", Gaming),
    ("ea", Gaming),
];

/// Whole-word (or phrase) match; a plural "s" after the needle still counts ("credit cards").
fn word_hit(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    let boundary = |i: usize| i >= hay.len() || !(bytes[i] as char).is_alphanumeric();
    hay.match_indices(needle).any(|(i, m)| {
        let before = i == 0 || !(bytes[i - 1] as char).is_alphanumeric();
        let end = i + m.len();
        before && (boundary(end) || (bytes.get(end) == Some(&b's') && boundary(end + 1)))
    })
}

/// Brand from a known site named in a title ("Coinbase" -> coinbase.com). Brands shorter than
/// four letters ("ea", "aa", "x") are skipped: too likely to appear inside unrelated titles.
fn brand_category(title: &str) -> Option<Category> {
    let lower = title.to_lowercase();
    let mut best: Option<(usize, Category)> = None;
    for &(d, c) in DOMAINS {
        if d.contains('/') || c == Uncategorized {
            continue;
        }
        let labels: Vec<&str> = d.split('.').collect();
        let brand = if labels.len() >= 2 {
            labels[labels.len() - 2]
        } else {
            labels[0]
        };
        if brand.len() >= 4
            && word_hit(&lower, brand)
            && best.is_none_or(|(len, _)| brand.len() > len)
        {
            best = Some((brand.len(), c));
        }
    }
    best.map(|(_, c)| c)
}

fn keyword_category(text: &str) -> Option<Category> {
    let lower = text.to_lowercase();
    // Longer phrases first, so "credit union" wins over "credit" and "store card" over "store".
    let mut best: Option<(usize, Category)> = None;
    for &(k, c) in KEYWORDS {
        if word_hit(&lower, k) && best.is_none_or(|(len, _)| k.len() > len) {
            best = Some((k.len(), c));
        }
    }
    best.map(|(_, c)| c)
}

fn is_private_address(host: &str) -> bool {
    let parts: Vec<u8> = host.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 4 || host.split('.').count() != 4 {
        return host == "localhost" || host.ends_with(".local") || host.ends_with(".lan");
    }
    matches!(
        parts.as_slice(),
        [10, ..] | [192, 168, ..] | [172, 16..=31, ..] | [127, ..]
    )
}

/// Category from a URL: known site (including path-specific entries), or a private address.
pub fn website_category(url: &str) -> Option<Category> {
    let host = host_of(url).trim_start_matches("www.").to_lowercase();
    if host.is_empty() {
        return None;
    }
    if is_private_address(&host) {
        return Some(Devices);
    }
    let rest = url
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(url)
        .to_lowercase();
    let rest = rest.trim_start_matches("www.");
    // Most specific match wins: longest domain (or domain/path) suffix.
    let mut best: Option<(usize, Category)> = None;
    for &(d, c) in DOMAINS {
        let dotted_suffix = |hay: &str| {
            hay.len() > d.len()
                && hay.ends_with(d)
                && hay.as_bytes()[hay.len() - d.len() - 1] == b'.'
        };
        let hit = if d.contains('/') {
            rest.starts_with(d)
                || rest
                    .match_indices(d)
                    .any(|(i, _)| i > 0 && rest.as_bytes()[i - 1] == b'.')
        } else {
            host == d || dotted_suffix(&host)
        };
        if hit && best.is_none_or(|(len, _)| d.len() > len) {
            best = Some((d.len(), c));
        }
    }
    best.map(|(_, c)| c)
}

pub fn manual_category(e: &Entry) -> Option<Category> {
    match e.custom_data.get(CATEGORY_KEY)?.value.as_ref()? {
        CustomDataValue::String(s) => Category::from_key(s.strip_prefix(VALUE_PREFIX).unwrap_or(s)),
        // A bare key that the crate decoded as base64: re-encode to recover the text.
        CustomDataValue::Binary(b) => {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            Category::from_key(&STANDARD.encode(b))
        }
    }
}

/// Sets (Some) or clears (None, back to automatic) the user's category for an entry.
pub fn set_manual_category(e: &mut Entry, category: Option<Category>) {
    match category {
        Some(c) => {
            e.custom_data.insert(
                CATEGORY_KEY.to_owned(),
                CustomDataItem {
                    value: Some(CustomDataValue::String(format!(
                        "{VALUE_PREFIX}{}",
                        c.key()
                    ))),
                    last_modification_time: Some(Times::now()),
                },
            );
        }
        None => {
            e.custom_data.remove(CATEGORY_KEY);
        }
    }
}

/// Classifies an entry given the names of the groups it sits in (innermost first).
pub fn classify_with_groups(e: &Entry, groups: &[&str]) -> (Category, Source) {
    if let Some(c) = manual_category(e) {
        return (c, Source::Manual);
    }
    let url = e.get_url().unwrap_or("");
    if let Some(c) = website_category(url) {
        // A known site that is not itself categorised (e.g. a charity) falls through to the title.
        if c != Uncategorized {
            return (c, Source::Website);
        }
    }
    let title = e.get_title().unwrap_or("");
    if let Some(c) = brand_category(title).or_else(|| keyword_category(title)) {
        return (c, Source::Title);
    }
    for g in groups {
        if let Some(c) = keyword_category(g) {
            return (c, Source::Group);
        }
    }
    // Last hint: words inside the host itself ("mybank-portal.com").
    let host = host_of(url).replace(['.', '-'], " ");
    if let Some(c) = keyword_category(&host) {
        return (c, Source::Website);
    }
    (Uncategorized, Source::None)
}

/// Classifies an entry in its database, using its group path as a hint.
pub fn classify(e: &EntryRef<'_>) -> (Category, Source) {
    let db = e.database();
    let mut names: Vec<String> = Vec::new();
    let mut id = Some(e.parent().id());
    let root = db.root().id();
    while let Some(gid) = id {
        if gid == root {
            break;
        }
        let Some(g) = db.group(gid) else { break };
        names.push(g.name.clone());
        id = g.parent().map(|p| p.id());
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    classify_with_groups(e, &refs)
}

#[cfg(test)]
mod tests {
    use keepass::Database;
    use keepass::db::fields;

    use super::*;

    fn cat(title: &str, url: &str, groups: &[&str]) -> Category {
        let mut db = Database::new();
        let id = db
            .root_mut()
            .add_entry()
            .edit(|e| {
                e.set_unprotected(fields::TITLE, title);
                e.set_unprotected(fields::URL, url);
            })
            .id();
        classify_with_groups(&db.entry(id).unwrap(), groups).0
    }

    /// Entries modelled on a real vault's titles and sites.
    #[test]
    fn real_world_entries() {
        let cases = [
            ("23andme (closed)", "https://www.23andme.com/", Health),
            ("Affirm", "https://www.affirm.com/", Loans),
            ("Airbnb", "https://www.airbnb.com/", Travel),
            (
                "Alaska Airlines Mileage",
                "https://www.alaskaair.com/",
                Travel,
            ),
            ("Amazon", "https://www.amazon.com/", Shopping),
            ("American Airlines", "https://aa.com", Travel),
            ("Apple Developer", "https://developer.apple.com", Developer),
            ("Apple ID", "https://card.apple.com", CreditCards),
            (
                "AquaTru (Water Filter)",
                "https://shop.aquatruwater.com",
                Shopping,
            ),
            ("ASUS RT-BE92U (WiFi)", "http://192.168.50.1", Devices),
            (
                "Azure Text-To-Speech",
                "https://portal.azure.com",
                Developer,
            ),
            ("Bookmap", "https://portal.bookmap.com", Investing),
            (
                "Broadcom (VMware Workstation Pro)",
                "https://support.broadcom.com",
                Software,
            ),
            ("Coinbase", "", Investing),
            ("Coinrule", "https://web.coinrule.com", Investing),
            ("EA Games", "", Gaming),
            ("Elements Massage", "https://elementsmassage.com", Health),
            (
                "Florida DMV Portal",
                "https://mydmvportal.flhsmv.gov",
                Government,
            ),
            (
                "FPL (Florida Power & Light) Electic Company",
                "https://www.fpl.com",
                Bills,
            ),
            (
                "Spotify Developer",
                "https://developer.spotify.com/",
                Developer,
            ),
            ("Chase Freedom", "https://www.chase.com", Banking),
            ("Navy Federal Credit Union", "", Banking),
            ("Experian", "https://www.experian.com", CreditBureaus),
            ("Netflix", "https://netflix.com", Streaming),
        ];
        for (title, url, expected) in cases {
            assert_eq!(cat(title, url, &[]), expected, "{title} / {url}");
        }
    }

    #[test]
    fn group_names_are_a_fallback_hint() {
        assert_eq!(cat("Main account", "", &["Savings"]), Banking);
        assert_eq!(cat("Platinum", "", &["Credit Cards"]), CreditCards);
        assert_eq!(cat("Something", "", &["RTX (Raytheon)"]), Jobs);
        assert_eq!(cat("Unknown thing", "", &["Misc"]), Uncategorized);
        // A known site beats the group.
        assert_eq!(cat("Netflix", "https://netflix.com", &["Bills"]), Streaming);
    }

    #[test]
    fn longest_keyword_wins() {
        assert_eq!(keyword_category("Navy Federal Credit Union"), Some(Banking));
        assert_eq!(keyword_category("Student Loan Servicer"), Some(Loans));
        assert_eq!(
            keyword_category("Cardiology clinic"),
            Some(Health),
            "'card' must be a whole word"
        );
    }

    #[test]
    fn manual_category_overrides_and_clears() {
        let mut db = Database::new();
        let id = db
            .root_mut()
            .add_entry()
            .edit(|e| e.set_unprotected(fields::URL, "https://amazon.com"))
            .id();
        set_manual_category(&mut db.entry_mut(id).unwrap(), Some(Bills));
        assert_eq!(
            classify_with_groups(&db.entry(id).unwrap(), &[]),
            (Bills, Source::Manual)
        );
        set_manual_category(&mut db.entry_mut(id).unwrap(), None);
        assert_eq!(
            classify_with_groups(&db.entry(id).unwrap(), &[]),
            (Shopping, Source::Website)
        );
    }

    #[test]
    fn private_addresses_are_devices() {
        assert_eq!(website_category("http://10.0.0.1/admin"), Some(Devices));
        assert_eq!(website_category("https://172.20.1.5"), Some(Devices));
        assert_eq!(website_category("http://nas.local:5000"), Some(Devices));
        assert_eq!(website_category("https://8.8.8.8"), None);
    }

    #[test]
    fn manual_category_survives_a_save_and_reload() {
        let mut db = Database::new();
        db.config.kdf_config = keepass::config::KdfConfig::Aes { rounds: 1000 };
        let id = db.root_mut().add_entry().id();
        set_manual_category(&mut db.entry_mut(id).unwrap(), Some(Shopping));
        let key = keepass::DatabaseKey::new().with_password("pw");
        let mut buf = Vec::new();
        db.save(&mut buf, key.clone()).unwrap();
        let back = Database::parse(&buf, key).unwrap();
        assert_eq!(manual_category(&back.entry(id).unwrap()), Some(Shopping));
    }

    #[test]
    fn bare_keys_read_back_as_binary_are_recovered() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let mut db = Database::new();
        let id = db.root_mut().add_entry().id();
        db.entry_mut(id).unwrap().custom_data.insert(
            CATEGORY_KEY.into(),
            CustomDataItem {
                value: Some(CustomDataValue::Binary(
                    STANDARD.decode("shopping").unwrap(),
                )),
                last_modification_time: None,
            },
        );
        assert_eq!(manual_category(&db.entry(id).unwrap()), Some(Shopping));
    }

    #[test]
    fn keys_round_trip() {
        for c in Category::ALL {
            assert_eq!(Category::from_key(c.key()), Some(c));
        }
    }
}
