//! The steps one capture runs: a one-file page (PDF or single HTML), design, structure, media, each page format,
//! and the Before/After compare. The run turns a failed design, structure, media or page step into a skip reason;
//! a failed one-file page or compare fails the capture.

use serde_json::{Map, Value, json};

use crate::capture::design;
use crate::capture::{
    Before, PageFormat, Request, Target, What, design_md, media, page, shot, structure, tokens_json,
};
use crate::driver::{ActError, Browser, failed};
use crate::scripts;

use super::answer::{answer, shot_json};
use super::record::pretty;
use super::{HTML_CAP, OUTCOME_KEY, PNG_CAP, Run, SMALL_CAP, TEXT_CAP};

impl Browser {
    pub(super) async fn capture_one_file(
        &self,
        session_id: &str,
        request: &Request,
        target: &Target<'_>,
        mut run: Run,
    ) -> Result<Value, ActError> {
        let what = request.what;
        // One-file choices do not add a manifest. Produce everything before committing any output.
        let mut outputs = Vec::new();
        let mut kept = 0;
        for format in &request.formats {
            let (bytes, links) = crate::capture::one_file::produce(
                self,
                session_id,
                *format,
                target.fetch,
                &run.source.url,
                &run.source.captured_at,
            )
            .await
            .map_err(failed)?;
            let name = format.file();
            outputs.push((name, bytes));
            kept += links;
        }
        for (name, bytes) in outputs {
            run.folder
                .write_atomic(name, &bytes, crate::capture::single_html::MAX_TOTAL as u64)
                .map_err(failed)?;
        }
        let mut result = answer(&run, what);
        result["keptAsLinks"] = json!(kept);
        if let [file] = run.folder.written() {
            result["path"] = json!(run.folder.path.join(&file.name).display().to_string());
        }
        result[OUTCOME_KEY] = json!(format!(
            "saved {} to {}/",
            run.folder
                .written()
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            run.folder.relative
        ));
        Ok(result)
    }

    pub(super) async fn capture_design(
        &self,
        session_id: &str,
        run: &mut Run,
    ) -> Result<(), String> {
        let page = self.page_of(session_id).await?;
        let stats: design::Stats = self
            .run_script(session_id, scripts::design(&page, 4000))
            .await?;
        self.check_answer(session_id, &page, &stats.url)?;
        let tokens = design::derive(&stats);
        run.folder.write(
            "DESIGN.md",
            design_md(&tokens, &run.source).as_bytes(),
            SMALL_CAP,
        )?;
        run.folder.write(
            "tokens.json",
            &pretty(&tokens_json(&tokens, &run.source)),
            SMALL_CAP,
        )?;
        let colours: Map<String, Value> = tokens
            .colours
            .iter()
            .map(|c| (c.name.clone(), json!(c.value.hex())))
            .collect();
        let levels: Vec<String> = tokens
            .typography
            .iter()
            .map(|t| format!("{} {}px/{}", t.name, design::trim_number(t.size), t.weight))
            .collect();
        run.fonts = tokens.typography.iter().map(|t| t.family.clone()).collect();
        run.fonts.dedup();
        run.summary.insert(
            "design".to_owned(),
            json!({"theme": tokens.theme, "colors": colours, "typeLevels": levels,
                   "spacingBase": format!("{}px", design::trim_number(tokens.base)),
                   "contrastWarnings": tokens.warnings.len(), "monochrome": tokens.monochrome}),
        );
        Ok(())
    }

    pub(super) async fn capture_structure(
        &self,
        session_id: &str,
        run: &mut Run,
    ) -> Result<(), String> {
        let page = self.page_of(session_id).await?;
        let found: structure::Page = self
            .run_script(session_id, scripts::structure(&page, 1500))
            .await?;
        self.check_answer(session_id, &page, &found.url)?;
        let at = run.source.captured_at.clone();
        run.folder.write(
            "elements.json",
            &pretty(&structure::elements_json(&found, &at)),
            TEXT_CAP,
        )?;
        run.folder.write(
            "summary.md",
            structure::summary_md(&found, &at).as_bytes(),
            SMALL_CAP,
        )?;
        for e in found.elements.iter().filter(|e| e.level.is_some()).take(30) {
            if let Some(text) = &e.text {
                run.page_words.push(("heading".to_owned(), text.clone()));
            }
        }
        run.summary.insert(
            "structure".to_owned(),
            json!({"elements": found.elements.len(), "counts": found.counts, "truncated": found.truncated}),
        );
        Ok(())
    }

    pub(super) async fn capture_media(
        &self,
        session_id: &str,
        run: &mut Run,
        target: &Target<'_>,
    ) -> Result<(), String> {
        let page = self.page_of(session_id).await?;
        let listing: media::Listing = self
            .run_script(session_id, scripts::media(&page, 200))
            .await?;
        self.check_answer(session_id, &page, &listing.url)?;
        let entries = media::save(&mut run.folder, &listing, session_id, target.fetch).await?;
        let mut statuses: Map<String, Value> = Map::new();
        for e in &entries {
            let status = e["status"].as_str().unwrap_or("listed").to_owned();
            let n = statuses.get(&status).and_then(Value::as_u64).unwrap_or(0);
            statuses.insert(status, json!(n + 1));
        }
        let manifest = json!({
            "schema": "vibe-work/capture-media@1",
            "source": {"url": run.source.url, "capturedAt": run.source.captured_at},
            "note": "Captured logos, images and fonts are for reference only: the site's licence applies. Only files \
                     the page had loaded were fetched, without cookies.",
            "faces": listing.faces,
            "items": entries,
        });
        run.folder
            .write("media.json", &pretty(&manifest), SMALL_CAP)?;
        for item in listing.items.iter().filter(|i| i.kind == "logo").take(5) {
            if let Some(alt) = &item.alt {
                run.page_words.push(("logo".to_owned(), alt.clone()));
            }
        }
        run.summary.insert(
            "media".to_owned(),
            json!({"listed": entries.len(), "byStatus": statuses}),
        );
        Ok(())
    }

    pub(super) async fn capture_page(
        &self,
        session_id: &str,
        run: &mut Run,
        format: PageFormat,
        method: Option<shot::Method>,
    ) -> Result<(), String> {
        let entry = match format {
            PageFormat::Text(f) => {
                let (text, cut) = page::read(self, session_id, f).await?;
                let body = page::file_text(f, &text, &run.source.url, &run.source.captured_at, cut);
                let cap = if f == page::Format::Html {
                    HTML_CAP
                } else {
                    TEXT_CAP
                };
                let w = run.folder.write(f.file(), body.as_bytes(), cap)?;
                let mut entry = json!({"file": w.name, "bytes": w.bytes, "cut": cut});
                if f == page::Format::Html {
                    // Said in the answer too, not only inside the file.
                    entry["note"] = json!(page::OPENS_SITE.trim());
                }
                entry
            }
            PageFormat::Pdf | PageFormat::SingleHtml => {
                return Err("one-file formats require a page capture".into());
            }
            PageFormat::Png => {
                let full = shot::full_page(self, session_id, method).await?;
                let w = run.folder.write("page.png", &full.png, PNG_CAP)?;
                shot_json(&full, &w.name)
            }
        };
        let page = run.summary.entry("page").or_insert_with(|| json!({}));
        let key = match format {
            PageFormat::Text(f) => f.file(),
            PageFormat::Png => "page.png",
            PageFormat::Pdf => "page.pdf",
            PageFormat::SingleHtml => "page-one-file.html",
        };
        page[key] = entry;
        Ok(())
    }

    pub(super) async fn capture_compare(
        &self,
        session_id: &str,
        run: &mut Run,
        what: What,
        method: Option<shot::Method>,
    ) -> Result<(), String> {
        if what == What::Before {
            let full = shot::full_page(self, session_id, method).await?;
            let w = run.folder.write("before.png", &full.png, PNG_CAP)?;
            run.before = Some(Before {
                relative: run.folder.relative.clone(),
                url: run.source.url.clone(),
                method: full.method,
            });
            run.summary
                .insert("before".to_owned(), shot_json(&full, &w.name));
            return Ok(());
        }
        let before = self
            .captures()
            .before(session_id)
            .ok_or("there is no before shot in this chat")?;
        let full = shot::full_page(self, session_id, Some(before.method)).await?;
        let w = run.folder.write("after.png", &full.png, PNG_CAP)?;
        let earlier = run.folder.read("before.png", PNG_CAP)?;
        let comparison = shot::compare(&earlier, &full.png)?;
        run.folder
            .write("diff.png", &comparison.diff_png, PNG_CAP)?;
        let record = json!({
            "schema": "vibe-work/capture-compare@1",
            "before": {"url": before.url, "file": "before.png"},
            "after": {"url": run.source.url, "file": w.name, "capturedAt": run.source.captured_at},
            "comparison": comparison,
            "note": "changedShare: share of the common area whose pixels changed; regions: changed areas (x, y, \
                     width, height in pixels of after.png), largest first; diff.png shows them in red.",
        });
        run.folder
            .write("compare.json", &pretty(&record), SMALL_CAP)?;
        run.summary.insert(
            "compare".to_owned(),
            json!({"changedShare": comparison.changed_share, "regions": comparison.regions,
                   "beforeSize": comparison.before_size, "afterSize": comparison.after_size,
                   "samePage": before.url == run.source.url}),
        );
        Ok(())
    }
}
