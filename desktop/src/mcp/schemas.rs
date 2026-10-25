use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn tools() -> Vec<Tool> {
    let id = json!({"type":"integer","minimum":1});
    let ids = json!({"type":"array","items":id,"minItems":1,"maxItems":10000,"uniqueItems":true});
    let revision = json!({"type":"integer","minimum":0});
    let text = json!({"type":"string"});
    let query = object(
        json!({
            "ids":ids,"text":text,"extensions":{"type":"array","items":text},
            "rating":{"type":"integer","minimum":0,"maximum":5},"rating_mode":{"enum":["gte","eq","lte"]},
            "flags":{"type":"array","items":{"enum":[-1,0,1]}},"labels":{"type":"array","items":{"enum":["","red","yellow","green","blue","purple"]}},
            "metadata":{"type":"object","additionalProperties":{"type":"array","items":text}},
            "root_id":id,"folder":text,"tags":{"type":"array","items":text},
            "added_from":{"type":"integer","description":"Unix milliseconds"},"added_to":{"type":"integer","description":"Unix milliseconds"},
            "include_missing":{"type":"boolean"},"sort":{"enum":["name","added","modified","size"]},"descending":{"type":"boolean"}
        }),
        &[],
    );
    let defaults = serde_json::to_value(advanced_explorer_develop::Settings::default()).unwrap();
    let properties = defaults
        .as_object()
        .unwrap()
        .iter()
        .filter(|(name, _)| name.as_str() != "version")
        .map(|(name, value)| {
            let kind = match value {
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::Array(_) => "array",
                Value::String(_) => "string",
                _ => "object",
            };
            (
                name.clone(),
                if value.is_null() {
                    json!({"type":["number","null"],"default":value})
                } else {
                    json!({"type":kind,"default":value})
                },
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let mut patch = object(Value::Object(properties), &[]);
    patch["minProperties"] = json!(1);
    patch["description"] = json!(
        "Top-level development fields to replace; omitted fields stay unchanged. Exposure: -5..5 EV. Use get_develop_settings for current values. Arrays replace the entire field. Values are validated by OpenRoom."
    );
    let border = object(
        json!({
            "color":{"enum":["white","black"]},
            "aspect_ratio":{"anyOf":[{"const":0},{"type":"number","minimum":0.5,"maximum":2}],"description":"Finished width / height; 0 keeps the photo ratio. Adds at least 4% short-edge margin on all sides."}
        }),
        &["color", "aspect_ratio"],
    );
    let options = object(
        json!({"quality":{"type":"integer","minimum":1,"maximum":100,"default":92},"long_edge":{"type":"integer","minimum":0,"maximum":65535,"default":0},"border":{"anyOf":[{"type":"null"},border]}}),
        &[],
    );
    let definitions = vec![
        (
            "get_app_state",
            "Read the open catalog, selection, view, filters, unsaved state and MCP jobs.",
            json!({}),
            vec![],
        ),
        (
            "select_photos",
            "Show these catalog photos as an AI selection and select the first ID. Replaces the visible filter; the user can clear it.",
            json!({"ids":{"type":"array","items":id,"minItems":1,"maxItems":200,"uniqueItems":true}}),
            vec!["ids"],
        ),
        (
            "set_view",
            "Switch the application to grid, loupe, compare, survey or develop.",
            json!({"view":{"enum":["grid","loupe","compare","survey","develop"]}}),
            vec!["view"],
        ),
        (
            "list_roots",
            "List registered photo folders.",
            json!({}),
            vec![],
        ),
        (
            "list_folders",
            "List catalog folders and photo counts without scanning disk.",
            json!({}),
            vec![],
        ),
        (
            "search_photos",
            "Search saved catalog data. Return up to 200 photos and an opaque cursor. Reuse the same query with next; restart if the catalog changed.",
            json!({"query":query,"limit":{"type":"integer","minimum":1,"maximum":200,"default":200},"cursor":{"type":"object","description":"Pass next from the previous page unchanged"}}),
            vec![],
        ),
        (
            "count_photos",
            "Count photos matching saved catalog data.",
            json!({"query":query}),
            vec![],
        ),
        (
            "get_photo",
            "Get saved photo details, rating, tags and memo.",
            json!({"id":id}),
            vec!["id"],
        ),
        (
            "get_metadata",
            "Read photo metadata, including GPS when present. Only catalog photo IDs are accepted.",
            json!({"id":id}),
            vec!["id"],
        ),
        (
            "get_metadata_facets",
            "Get indexed metadata values and counts. indexing_required indicates incomplete metadata indexing.",
            json!({"query":query,"field":{"enum":["date","camera","lens","iso","aperture","focal","label","extension"]}}),
            vec!["field"],
        ),
        (
            "add_root",
            "Register an absolute photo folder without moving originals. Use start_scan to index it.",
            json!({"path":text}),
            vec!["path"],
        ),
        (
            "start_scan",
            "Start a folder scan. Omit root_id to scan all roots. Returns a job; poll get_job.",
            json!({"root_id":id}),
            vec![],
        ),
        (
            "start_metadata_index",
            "Index metadata in the query's root/folder scope. Other filters are ignored to include unindexed photos. Returns a job.",
            json!({"query":query}),
            vec![],
        ),
        (
            "update_photo_attributes",
            "Atomically update rating, flag or label. Omitted fields remain unchanged; an empty label clears it.",
            json!({"ids":ids,"rating":{"type":"integer","minimum":0,"maximum":5},"flag":{"enum":[-1,0,1]},"label":{"enum":["","red","yellow","green","blue","purple"]}}),
            vec!["ids"],
        ),
        (
            "update_annotations",
            "Atomically replace only supplied annotation fields for up to 500 photos. Empty tags or memo explicitly clears that field.",
            json!({"ids":{"type":"array","items":id,"minItems":1,"maxItems":500,"uniqueItems":true},"patch":object(json!({"tags":{"type":"array","items":text,"maxItems":100},"rating":{"type":"integer","minimum":0,"maximum":5},"memo":text}),&[])}),
            vec!["ids", "patch"],
        ),
        (
            "get_develop_settings",
            "Read saved development settings, revision and defaults. Unsaved GUI edits are reported separately by get_app_state.",
            json!({"id":id}),
            vec!["id"],
        ),
        (
            "preview_photo",
            "Return a JPEG image using saved settings plus an optional temporary patch. Does not save. Uses a 384 or 1600 pixel working image.",
            json!({"id":id,"patch":patch,"bound":{"enum":[384,1600],"default":1600}}),
            vec!["id"],
        ),
        (
            "update_develop_settings",
            "Apply a partial development edit and append history. expected_revision must match; reload after conflicts.",
            json!({"id":id,"expected_revision":revision,"patch":patch}),
            vec!["id", "expected_revision", "patch"],
        ),
        (
            "list_presets",
            "List saved development presets and their partial settings.",
            json!({}),
            vec![],
        ),
        (
            "save_preset",
            "Save a named partial development preset.",
            json!({"name":text,"patch":patch}),
            vec!["name", "patch"],
        ),
        (
            "apply_preset",
            "Apply a preset to up to 500 photos atomically. Every target requires its current revision. Adds history without enabling GUI auto-sync.",
            json!({"preset_id":id,"targets":{"type":"array","minItems":1,"maxItems":500,"items":object(json!({"id":id,"expected_revision":revision}),&["id","expected_revision"])}}),
            vec!["preset_id", "targets"],
        ),
        (
            "list_develop_history",
            "List up to 100 history entries, newest first. Pass the last history ID as before for older entries.",
            json!({"id":id,"before":id}),
            vec!["id"],
        ),
        (
            "restore_develop_history",
            "Restore an entry as a new edit; preserve later history and reject stale revisions.",
            json!({"id":id,"history_id":id,"expected_revision":revision}),
            vec!["id", "history_id", "expected_revision"],
        ),
        (
            "start_export",
            "Export saved settings to an existing absolute directory as <name>-<id>-edited.<ext>. Never overwrite files. JPEG or 16-bit PNG/TIFF, SDR sRGB, without source EXIF/GPS. Returns a job; check errors as well as status.",
            json!({"ids":ids,"destination":text,"format":{"enum":["jpeg","png","tiff"]},"options":options}),
            vec!["ids", "destination", "format"],
        ),
        (
            "get_job",
            "Read progress and final results. Jobs are retained for this application session, up to 100 entries.",
            json!({"job_id":text}),
            vec!["job_id"],
        ),
        (
            "cancel_job",
            "Request cancellation. Already committed edits or exported files remain; poll until the job stops.",
            json!({"job_id":text}),
            vec!["job_id"],
        ),
    ];
    definitions
        .into_iter()
        .map(|(name, description, properties, required)| {
            let schema = object(properties, &required).as_object().unwrap().clone();
            let mut tool = Tool::new(name, description, schema);
            let read_only = !super::operations::is_mutation(name)
                && !matches!(name, "select_photos" | "set_view" | "cancel_job");
            tool.annotations = Some(
                ToolAnnotations::new()
                    .read_only(read_only)
                    .destructive(!read_only)
                    .open_world(false),
            );
            tool
        })
        .collect()
}
