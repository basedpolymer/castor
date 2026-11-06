//! OCR with Tesseract (the copy built into MuPDF). Trained data (tessdata_fast, Apache-2.0) is
//! downloaded on demand per language into `downloads_dir()/tessdata/`.
//!
//! MuPDF's Tesseract glue routes Leptonica's allocations through a single global, so only one
//! OCR engine may exist in the process at a time: [`Engine`] holds a process-wide lock.

use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::path::PathBuf;

use anyhow::Result;
use castor_core::download::{self, Asset};
use castor_core::RunCx;

use crate::ffi;

/// A recognised word. `bbox` = x0, y0, x1, y1 in pixels of the image given to `recognize`.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub bbox: [f32; 4],
    /// 0..100.
    pub conf: f32,
}

/// The result for one image.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub words: Vec<Word>,
    /// Reading order, `\n` between lines, an empty line between paragraphs.
    pub text: String,
}

const COMMIT: &str = "87416418657359cb625c412a48b6e1d6d41c29bd";
macro_rules! td {
    ($code:literal, $sha:literal, $size:literal) => {
        Asset {
            id: concat!("tessdata/", $code, ".traineddata"),
            url: concat!(
                "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd/",
                $code,
                ".traineddata"
            ),
            sha256: $sha,
            size: $size,
            license: "Apache-2.0",
        }
    };
}

/// Every downloadable language model (tessdata_fast at commit [`COMMIT`]).
pub static CATALOG: &[Asset] = &[
    td!("eng", "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2", 4113088),
    td!("fra", "ced037562e8c80c13122dece28dd477d399af80911a28791a66a63ac1e3445ca", 1130365),
    td!("deu", "19d219bbb6672c869d20a9636c6816a81eb9a71796cb93ebe0cb1530e2cdb22d", 1525436),
    td!("spa", "6f2e04d02774a18f01bed44b1111f2cd7f3ba7ac9dc4373cd3f898a40ea6b464", 2294433),
    td!("ita", "b8f89e1e785118dac4d51ae042c029a64edb5c3ee42ef73027a6d412748d8827", 2701314),
    td!("por", "c4932b937207a9514b7514d518b931a99938c02a28a5a5a553f8599ed58b7deb", 1982756),
    td!("nld", "ced0e5e046a84c908a6aa7accbef9a232c4a5d9a8276691b81c6ee64d02963f6", 6050296),
    td!("pol", "c4476cdbc0e33d898d32345122b7be1cbf85ace15f920f06c7714756e1ef79b2", 4765518),
    td!("rus", "e16e5e036cce1d9ec2b00063cf8b54472625b9e14d893a169e2b0dedeb4df225", 3861738),
    td!("ukr", "d59e53e2bded32f4445f124b4b00240fcac7e8044c003ab822ccb94f0b3db59b", 3825102),
    td!("jpn", "1f5de9236d2e85f5fdf4b3c500f2d4926f8d9449f28f5394472d9e8d83b91b4d", 2471260),
    td!("jpn_vert", "bf1e2640954691797e2dc14f38533e601b59ee37958698ae0f0b81dc6f09c71b", 3037480),
    td!("chi_sim", "a5fcb6f0db1e1d6d8522f39db4e848f05984669172e584e8d76b6b3141e1f730", 2469156),
    td!("chi_sim_vert", "20590de84725bab69cde93bd6e8ed360a13cc5421a7e7364ddeb93e9af53d6da", 1927902),
    td!("chi_tra", "529c5b5797d64b126065cd55f2bb4c7fd7b15790798091b1ff259941a829330b", 2366642),
    td!("chi_tra_vert", "1df02a4b210e5c217b783819538b63e9dfe6904e2b5e53b62664f1b9f7a989d0", 1824756),
    td!("kor", "6b85e11d9bbf07863b97b3523b1b112844c43e713df8b66418a081fd1060b3b2", 1677415),
    td!("kor_vert", "c28f19dee36927baba5215fb793a3c00fff3ef2cfbcaa100122401d8f4374869", 1113715),
    td!("ara", "e3206d3dc87fd50c24a0fb9f01838615911d25168f4e64415244b67d2bb3e729", 1432056),
    td!("hin", "4c73ffc59d497c186b19d1e90f5d721d678ea6b2e277b719bee4e2af12271825", 1122751),
    td!("ces", "934bcaf97ef3348413263331131c9fa7f55f30db333c711929c124fb635f7e1b", 3795684),
    td!("slk", "fbcc400a9c74c6a13d922fcb1211b655d1b165387b675ed75cd2dbd756b974a5", 4427661),
    td!("slv", "b937632c17ce5fdf20535d25feec044da10a0c4c8234f4302d5606fd671cf60d", 3003829),
    td!("hrv", "9e515d9832ce259dbab550b1cc6b998f8b929faf2edacaaca981b05adb130571", 4103348),
    td!("srp", "aa41ae3d9cc705e60d398ab38a5c3cc8b772c0d420c7d4f0859beb13d0e321b6", 2149931),
    td!("srp_latn", "54b2e001bd8a336bdffc249c1487b8b7cb712f0a031f2551930ba050ebefa2a0", 3281787),
    td!("bos", "6cc8cc87cf1afbfa6a41febb725dbadb14bed96a46685906440a6eb8a7892f04", 2498242),
    td!("bul", "aebc9b0fcc8cfaf8a9f38a02bb7b85052bd850744696a2c11cf0081820e5b21e", 1675212),
    td!("ron", "9adfde6b51ba4b97efd10ea37c3070fd3fc2bad7815e81f5c3c198cd96216cc9", 2376323),
    td!("hun", "35067e7cfe102dcdc953f9a758fdfaa6296b17a1ee6d874ee780fa306430b9fb", 5296273),
    td!("fin", "61a04cd62b507c3d9ae0e1cda399e6715ebf49dea9df47897c8acdcd3bd3e13c", 7865732),
    td!("swe", "f7304988d41f833efebcc2d529df54b1903ecebbc3da1faabd19a0fddd4fe586", 4167034),
    td!("nor", "0451eb4f8049ae78196806bf878a389a2f40f1386fe038568cf4441226ba6ef2", 3610079),
    td!("dan", "acb1fd074487a31d1294fcdfd7d7c673467ffd8aeacb2ccd61ebcbf04eb4e2fa", 2580059),
    td!("isl", "5ec828c363f3f0062c3caf08216677cf5f3c0c981b05fab445123a16b98f10a9", 2278973),
    td!("est", "515d4a773682b286369511e83fe412bcff16a92a886f99c761c1d760a7e30456", 4458101),
    td!("lav", "7d9eaf22254b381f18b806cd6cb647862a397a7221adbce3e22cc421793a8847", 2717247),
    td!("lit", "1e383df5b055583bc01cb5764ecdf74c540753f2cb3f8205e7105361da4bc989", 3154896),
    td!("ell", "4fba8a0b461038d51f1c20d043d4f2ac38c4e778f1b90830847f7bd8fa3ba726", 1419514),
    td!("tur", "7393381111e1152420fc4092cb44eef4237580d21b92bf30d7d221aad192c6b7", 4550554),
    td!("heb", "11f9e43ab227f786352a50f75c94c2e9906f1baba86d93276da19da7ce0904db", 961404),
    td!("fas", "db1c0a91208aff00d3cf1ed2c1d23f76419afd5f024688b4f71adc3f2ce4a505", 431500),
    td!("urd", "62e8250ce2a994106e313a82e26a516a39e2cf159d0ce3c5b5008387fd0d555f", 1398718),
    td!("ben", "31163084c279aaebd376216f0c3d5c17ad4b5fee8db49dae79c20000b5de5964", 855841),
    td!("tam", "d02fbec24be4b07e32e80d0ccfc3b6b67a3c5d61c9d0a7c8532677990912c6ec", 3237963),
    td!("tel", "d10691fddd5b67802e1c12800ebb321d3b8bcd8d24a2ac3ff206f93188c04ab5", 2769654),
    td!("tha", "294227cc2d1292b0acb28d61d4115c88252b96d466ca90b417cf4cf0c67bf07c", 1072600),
    td!("vie", "79df64caf7bcfb2a27df5042ecb6121e196eada34da774956995747636d5bfa1", 531275),
    td!("ind", "69786901da87ab8766c1ea7fbb10b28f2110c14da3f6c8f2735df131fba95d88", 1122661),
    td!("msa", "e41a3e5febfec50c90371eb1cbb17a48b10cad387900e3420b1f134c1b766cba", 1747801),
    td!("cat", "250db73cd5b380d2798581295dc12f20d0828cdb335a65d833d12dfdbf57117d", 1146012),
    td!("eus", "40e7418296c355d9fd9ca843d115c51e740089e576e887443e61704823cd6624", 5175921),
    td!("glg", "7947619c5544d86849f563bd737ad90dbea9f5319fbd838c4747a7a1cd40b260", 2554555),
    td!("gle", "2fe9ba6119aac7e2a20d6cfb69ed91afe9520f41bc7e3a903f84280f2663858d", 1181824),
    td!("cym", "7f6ee3374749645a7c92dfe773f5c3d6492194d371712ecfd775edc53c363fb4", 2208919),
    td!("afr", "126d480bfae95be2a911ed4916465e27bde75fea2da631e21b96762e5f239646", 2652786),
    td!("swa", "395439d1ec308535066cbaea9b15e0e4cc81f8609170af76ab7e7e8d3ec42f3e", 2167651),
    td!("fil", "2b17f68014ab44b88b5e2334ebcd0ce0090d9af7c6fbef6fab22e0bbd9a934dd", 1841715),
    td!("kat", "557abb6f1c68bc1b286f1bdd00bb6b82f85a427a91899807dab6c2f6c7986731", 2524769),
    td!("hye", "b701d0d95799a716143dedb0197504e56f27a1bb133d6607ff5778c6988cb67c", 3463717),
    td!("aze", "a365310848aecb739f19369cb3831d4660fcd9345d798e91a3042455f9ccc9f0", 3524799),
    td!("kaz", "fcc01eed3815a42b9c6321c4c9d3606f39b166cbf95ade98b7d8d12063eae53d", 4734644),
    td!("uzb", "0e39021eefe692906b50d9bd22e580fa57fe43fa35db6846ad8b6369ddc21ea8", 6470703),
    td!("mon", "a151a3806d61ac43619cd383896d551ba5c3b07388ffec6fc83c8c604d677570", 2137042),
    td!("nep", "280ba9450b4f21afbf5985e0de87857b75a972577c50ef0603c141ddde4f1cb8", 1002911),
    td!("mar", "0ba3f2d116972e72fe9e176bc84c38e81dfb6670f4ed1f7f6c8e16a27da7cb61", 2118233),
    td!("guj", "fa69658614b4946a9afae8853d67e0689838803dfa3d12c2e35ec53ee6f8df34", 1418394),
    td!("pan", "1ec0907fc3534065ea9ae190c6bb7ec9e5c74fd9d2fa996aaec7407f11ad8131", 497721),
    td!("kan", "bd31e6b6ae93271e3bcf5383d306d8eefbb91542937cd6d735a5930c970e61d8", 3608331),
    td!("mal", "bd05cbf1b197e7810d2903419aedb06f9ef77bfedf50b358673c1d18d707cdb4", 5275996),
    td!("sin", "bc75c6df2375a30c9f7e759fa7d4b58ae3ecf9ce72668a702d01acb13e551422", 1727413),
    td!("khm", "47f110575341b322052f3becbefee61a3ecf1ef549352b5d4d33d28afe30d099", 1446926),
    td!("lao", "20124962e93e68121e02c49a949d1f9df5db87dd62e5e4aa578362ca532444f8", 6386744),
    td!("mya", "02aa6c25cfe9e583fa7b5d4131eac948f962308983f0f397df077dea58212b03", 4640561),
    td!("bel", "9c6668a0b202f3dcfe074b64620d108e1902ca7498a40b5a11b4a3da6112d58f", 3692098),
    td!("mkd", "58622bf154830fa62103359938564aeb8112b929759e22a48224f3ecfaac34c6", 1600188),
    td!("sqi", "1d89621d9afe8ddf35b403ecf8951edea52c3a960d241f287b1a7b2c5ba8daac", 1874705),
    td!("lat", "3859d8ba60404f4b79830622625bbc76fb4ee2808eac1ad360ffa77f0a533328", 3187463),
    td!("epo", "71181a6a07af3812aeedfa1aa993623424f4b8a6aac3e271b36ec11774e674d9", 4728393),
    td!("ltz", "5516a78efa30c050da7eb0727fec3de59670c19735c00678a67fdad98e4b3ec1", 2606426),
    td!("mlt", "3040b443d5d49e4183da0eaaf58d1033c3a616153d35d2342a56777f9bbb31c7", 2308796),
    td!("deu_latf", "7cd1b541e9d3884b9546a7d292c4f349cdb45ff646b4b43166dfc5099a8ad1a1", 6423052),
];

static LANGUAGES: &[(&str, &str)] = &[
    ("eng", "English"),
    ("fra", "Français"),
    ("deu", "Deutsch"),
    ("spa", "Español"),
    ("ita", "Italiano"),
    ("por", "Português"),
    ("nld", "Nederlands"),
    ("pol", "Polski"),
    ("rus", "Русский"),
    ("ukr", "Українська"),
    ("jpn", "日本語"),
    ("jpn_vert", "日本語（縦書き）"),
    ("chi_sim", "简体中文"),
    ("chi_sim_vert", "简体中文（竖排）"),
    ("chi_tra", "繁體中文"),
    ("chi_tra_vert", "繁體中文（直排）"),
    ("kor", "한국어"),
    ("kor_vert", "한국어 (세로쓰기)"),
    ("ara", "العربية"),
    ("hin", "हिन्दी"),
    ("ces", "Čeština"),
    ("slk", "Slovenčina"),
    ("slv", "Slovenščina"),
    ("hrv", "Hrvatski"),
    ("srp", "Српски"),
    ("srp_latn", "Srpski (latinica)"),
    ("bos", "Bosanski"),
    ("bul", "Български"),
    ("ron", "Română"),
    ("hun", "Magyar"),
    ("fin", "Suomi"),
    ("swe", "Svenska"),
    ("nor", "Norsk"),
    ("dan", "Dansk"),
    ("isl", "Íslenska"),
    ("est", "Eesti"),
    ("lav", "Latviešu"),
    ("lit", "Lietuvių"),
    ("ell", "Ελληνικά"),
    ("tur", "Türkçe"),
    ("heb", "עברית"),
    ("fas", "فارسی"),
    ("urd", "اردو"),
    ("ben", "বাংলা"),
    ("tam", "தமிழ்"),
    ("tel", "తెలుగు"),
    ("tha", "ไทย"),
    ("vie", "Tiếng Việt"),
    ("ind", "Bahasa Indonesia"),
    ("msa", "Bahasa Melayu"),
    ("cat", "Català"),
    ("eus", "Euskara"),
    ("glg", "Galego"),
    ("gle", "Gaeilge"),
    ("cym", "Cymraeg"),
    ("afr", "Afrikaans"),
    ("swa", "Kiswahili"),
    ("fil", "Filipino"),
    ("kat", "ქართული"),
    ("hye", "Հայերեն"),
    ("aze", "Azərbaycanca"),
    ("kaz", "Қазақша"),
    ("uzb", "Oʻzbekcha"),
    ("mon", "Монгол"),
    ("nep", "नेपाली"),
    ("mar", "मराठी"),
    ("guj", "ગુજરાતી"),
    ("pan", "ਪੰਜਾਬੀ"),
    ("kan", "ಕನ್ನಡ"),
    ("mal", "മലയാളം"),
    ("sin", "සිංහල"),
    ("khm", "ខ្មែរ"),
    ("lao", "ລາວ"),
    ("mya", "မြန်မာ"),
    ("bel", "Беларуская"),
    ("mkd", "Македонски"),
    ("sqi", "Shqip"),
    ("lat", "Latina"),
    ("epo", "Esperanto"),
    ("ltz", "Lëtzebuergesch"),
    ("mlt", "Malti"),
    ("deu_latf", "Deutsch (Fraktur)"),
];

/// (tesseract code, endonym), e.g. ("fra", "Français"). Used by `OptionSpec::language`.
pub fn languages() -> &'static [(&'static str, &'static str)] {
    LANGUAGES
}

/// The commit of tessdata_fast the catalog is pinned to.
pub fn tessdata_commit() -> &'static str {
    COMMIT
}

fn codes(langs: &str) -> Vec<&str> {
    let mut v: Vec<&str> = langs.split(['+', ',', ' ']).map(str::trim).filter(|s| !s.is_empty()).collect();
    v.dedup();
    if v.is_empty() {
        v.push("eng");
    }
    v
}

/// Assets to download for "fra+eng" (not yet present ones are what the app asks consent for).
/// Unknown codes are ignored here; [`recognize`] reports them.
pub fn assets(langs: &str) -> Vec<Asset> {
    codes(langs)
        .into_iter()
        .filter_map(|c| CATALOG.iter().find(|a| a.id == format!("tessdata/{c}.traineddata")).cloned())
        .collect()
}

fn tessdata_dir() -> PathBuf {
    castor_core::paths::downloads_dir().join("tessdata")
}

/// Make sure the trained data of `langs` is on disk (downloads what is missing).
pub fn ensure_languages(langs: &str, cx: &RunCx) -> Result<String> {
    let cs = codes(langs);
    for c in &cs {
        anyhow::ensure!(
            CATALOG.iter().any(|a| a.id == format!("tessdata/{c}.traineddata")),
            "{} '{c}'",
            castor_core::tr("pdf.err.ocr_language")
        );
    }
    for a in assets(langs) {
        if a.is_present() {
            continue;
        }
        cx.message(castor_core::tr_args("pdf.msg.downloading", &[("name", a.id.trim_start_matches("tessdata/"))]));
        download::ensure(&a, &cx.cancel_token(), &|done, total| {
            if total > 0 {
                cx.progress(done as f32 / total as f32 * 0.05);
            }
        })?;
    }
    Ok(cs.join("+"))
}

static OCR_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// A loaded Tesseract engine for some languages. Only one exists at a time in the process;
/// creating a second one waits (cancellable) for the first to be dropped.
pub struct Engine {
    h: *mut ffi::CastorOcr,
    _guard: parking_lot::MutexGuard<'static, ()>,
}

// The handle is only used through &mut self, from one thread at a time.
unsafe impl Send for Engine {}

/// A word with its line, as needed to rebuild text and place a text layer.
#[derive(Clone, Debug)]
pub struct OcrWord {
    pub text: String,
    /// Pixel box x0, y0, x1, y1.
    pub bbox: [f32; 4],
    pub conf: f32,
    pub line: usize,
    /// Line box (pixels): gives a common baseline/height for the words of a line.
    pub line_bbox: [f32; 4],
    /// Point size estimated by Tesseract (at the image dpi).
    pub pointsize: f32,
}

struct Collect {
    words: Vec<OcrWord>,
    cur_word: Option<[i32; 4]>,
    cur_line: Option<[i32; 4]>,
    line: usize,
    cancel: castor_core::CancelToken,
    progress: Box<dyn Fn(f32)>,
}

unsafe extern "C" fn on_char(arg: *mut c_void, unicode: c_int, line: *const c_int, word: *const c_int, _ch: *const c_int, pts: c_int) {
    let c = unsafe { &mut *(arg as *mut Collect) };
    let lb = unsafe { [*line, *line.add(1), *line.add(2), *line.add(3)] };
    let wb = unsafe { [*word, *word.add(1), *word.add(2), *word.add(3)] };
    let Some(ch) = char::from_u32(unicode as u32) else { return };
    if c.cur_line != Some(lb) {
        if c.cur_line.is_some() {
            c.line += 1;
        }
        c.cur_line = Some(lb);
        c.cur_word = None;
    }
    if c.cur_word != Some(wb) || c.words.is_empty() {
        c.cur_word = Some(wb);
        c.words.push(OcrWord {
            text: String::new(),
            bbox: wb.map(|v| v as f32),
            conf: 0.0,
            line: c.line,
            line_bbox: lb.map(|v| v as f32),
            pointsize: pts as f32,
        });
    }
    if let Some(w) = c.words.last_mut() {
        w.text.push(ch);
    }
}

unsafe extern "C" fn on_progress(arg: *mut c_void, percent: c_int) -> c_int {
    let c = unsafe { &*(arg as *const Collect) };
    (c.progress)(percent as f32 / 100.0);
    c.cancel.is_cancelled() as c_int
}

impl Engine {
    /// Load `langs` ("fra+eng"); downloads missing trained data first.
    pub fn new(langs: &str, cx: &RunCx) -> Result<Engine> {
        let langs = ensure_languages(langs, cx)?;
        let guard = loop {
            cx.check_cancel()?;
            if let Some(g) = OCR_LOCK.try_lock_for(std::time::Duration::from_millis(200)) {
                break g;
            }
            cx.message(castor_core::tr("pdf.msg.ocr_waiting"));
        };
        let dir = tessdata_dir();
        let clang = CString::new(langs)?;
        let cdir = CString::new(crate::path_str(&dir)?)?;
        let mut err = vec![0u8; 1024];
        ffi::ensure_base();
        let h = unsafe { ffi::castor_ocr_new(clang.as_ptr(), cdir.as_ptr(), err.as_mut_ptr().cast(), err.len() as i32) };
        anyhow::ensure!(!h.is_null(), "{}: {}", castor_core::tr("pdf.err.ocr_init"), ffi::err_string(&err));
        Ok(Engine { h, _guard: guard })
    }

    /// Recognise an 8-bit gray image. Returns words in reading order and the mean confidence.
    pub fn recognize_gray(
        &mut self,
        gray: &[u8],
        width: u32,
        height: u32,
        stride: usize,
        dpi: u32,
        cancel: castor_core::CancelToken,
        progress: impl Fn(f32) + 'static,
    ) -> Result<(Vec<OcrWord>, f32)> {
        anyhow::ensure!(width > 0 && height > 0 && gray.len() >= stride * height as usize, "empty image");
        let mut col =
            Collect { words: Vec::new(), cur_word: None, cur_line: None, line: 0, cancel: cancel.clone(), progress: Box::new(progress) };
        let mut confs: *mut c_int = std::ptr::null_mut();
        let mut nconfs: c_int = 0;
        let mut mean: c_int = 0;
        let mut err = vec![0u8; 1024];
        let rc = unsafe {
            ffi::castor_ocr_run(
                self.h,
                gray.as_ptr(),
                width as c_int,
                height as c_int,
                stride as c_int,
                dpi as c_int,
                on_char,
                on_progress,
                (&mut col as *mut Collect).cast(),
                &mut confs,
                &mut nconfs,
                &mut mean,
                err.as_mut_ptr().cast(),
                err.len() as i32,
            )
        };
        let conf_list: Vec<i32> =
            if confs.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(confs, nconfs as usize).to_vec() } };
        unsafe { ffi::castor_ocr_free_confs(confs) };
        if cancel.is_cancelled() {
            return Err(castor_core::job::Cancelled.into());
        }
        anyhow::ensure!(rc == 0, "OCR: {}", ffi::err_string(&err));
        let mut words: Vec<OcrWord> = col.words.into_iter().filter(|w| !w.text.trim().is_empty()).collect();
        let same = conf_list.len() == words.len();
        for (i, w) in words.iter_mut().enumerate() {
            w.text = w.text.trim().to_string();
            w.conf = if same { conf_list[i] as f32 } else { mean as f32 };
        }
        Ok((words, mean as f32))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { ffi::castor_ocr_free(self.h) };
    }
}

/// Rebuild reading-order text: words of a line joined by spaces, lines by `\n`, and an empty
/// line where the vertical gap suggests a new paragraph.
pub fn words_to_text(words: &[OcrWord]) -> String {
    let mut out = String::new();
    let mut prev: Option<(usize, [f32; 4])> = None;
    for w in words {
        match prev {
            Some((l, _)) if l == w.line => out.push(' '),
            Some((_, pb)) => {
                let h = (pb[3] - pb[1]).max(1.0);
                let gap = w.line_bbox[1] - pb[3];
                out.push('\n');
                if gap > h * 0.9 {
                    out.push('\n');
                }
            }
            None => {}
        }
        out.push_str(&w.text);
        prev = Some((w.line, w.line_bbox));
    }
    out
}

/// OCR an RGBA8 image. Downloads missing languages through `download::ensure`.
pub fn recognize(rgba: &[u8], width: u32, height: u32, langs: &str, cx: &RunCx) -> Result<Page> {
    anyhow::ensure!(rgba.len() >= width as usize * height as usize * 4, "image buffer too small");
    let gray: Vec<u8> = rgba
        .chunks_exact(4)
        .map(|p| {
            // Composite on white, then luma.
            let a = p[3] as u32;
            let mix = |c: u8| (c as u32 * a + 255 * (255 - a)) / 255;
            ((mix(p[0]) * 299 + mix(p[1]) * 587 + mix(p[2]) * 114) / 1000) as u8
        })
        .collect();
    let mut engine = Engine::new(langs, cx)?;
    let pcx = cx.clone();
    let (words, _) = engine.recognize_gray(&gray, width, height, width as usize, 300, cx.cancel_token(), move |f| {
        pcx.progress(0.05 + f * 0.95)
    })?;
    Ok(Page {
        text: words_to_text(&words),
        words: words.into_iter().map(|w| Word { text: w.text, bbox: w.bbox, conf: w.conf }).collect(),
    })
}
