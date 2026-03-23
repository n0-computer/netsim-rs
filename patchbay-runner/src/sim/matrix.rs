//! Matrix expansion for sim TOML files.
//!
//! A `[matrix]` table in a sim TOML defines axes whose Cartesian product generates
//! multiple simulation variants. Substitution operates on the parsed `toml::Value`
//! tree — the file is always valid TOML before and after expansion.
//!
//! See the spec in the repo root for full details.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use toml::Value;

/// A single matrix combination: maps placeholder names to replacement strings.
type SubstMap = HashMap<String, String>;

/// Per-axis parameter overrides: `axis_name -> variant_value -> { key -> value }`.
type AxisParams = HashMap<String, HashMap<String, HashMap<String, String>>>;

/// Extract the `[matrix]` table from a TOML value tree, returning the axis
/// definitions and per-axis params. The `[matrix]` key is removed from `root`.
///
/// Returns `None` if there is no `[matrix]` table.
fn extract_matrix(root: &mut toml::value::Table) -> Result<Option<MatrixDef>> {
    let Some(matrix_val) = root.remove("matrix") else {
        return Ok(None);
    };
    let Value::Table(mut matrix) = matrix_val else {
        bail!("`[matrix]` must be a table");
    };

    // Extract [matrix.params.*] before iterating axes.
    let params: AxisParams = if let Some(params_val) = matrix.remove("params") {
        parse_params_table(params_val)?
    } else {
        HashMap::new()
    };

    // Remaining keys are axes: each must be an array of strings.
    let mut axes: Vec<(String, Vec<String>)> = Vec::new();
    for (key, val) in &matrix {
        let Value::Array(arr) = val else {
            bail!("matrix axis `{key}` must be an array");
        };
        let mut values = Vec::new();
        for item in arr {
            let Value::String(s) = item else {
                bail!("matrix axis `{key}` values must be strings");
            };
            values.push(s.clone());
        }
        if values.is_empty() {
            bail!("matrix axis `{key}` must not be empty");
        }
        axes.push((key.clone(), values));
    }

    if axes.is_empty() {
        return Ok(None);
    }

    Ok(Some(MatrixDef { axes, params }))
}

struct MatrixDef {
    /// Ordered axes: `[(name, [value, ...])]`.
    axes: Vec<(String, Vec<String>)>,
    /// Per-axis params: `axis_name -> { variant_value -> { param_key -> param_value } }`.
    params: AxisParams,
}

/// Parse `[matrix.params]` which maps axis names to tables of variant params.
///
/// ```toml
/// [matrix.params.cond]
/// baseline = { latency = "0", rate = "0" }
/// impaired = { latency = "200", rate = "4000" }
/// ```
fn parse_params_table(val: Value) -> Result<AxisParams> {
    let Value::Table(axes) = val else {
        bail!("`[matrix.params]` must be a table");
    };
    let mut out = HashMap::new();
    for (axis_name, axis_val) in axes {
        let Value::Table(variants) = axis_val else {
            bail!("`[matrix.params.{axis_name}]` must be a table");
        };
        let mut variant_map = HashMap::new();
        for (variant_name, fields_val) in variants {
            let Value::Table(fields) = fields_val else {
                bail!("`[matrix.params.{axis_name}.{variant_name}]` must be a table");
            };
            let mut field_map = HashMap::new();
            for (k, v) in fields {
                let s = match v {
                    Value::String(s) => s,
                    Value::Integer(n) => n.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => bail!("matrix.params.{axis_name}.{variant_name}.{k} must be a scalar"),
                };
                field_map.insert(k, s);
            }
            variant_map.insert(variant_name, field_map);
        }
        out.insert(axis_name, variant_map);
    }
    Ok(out)
}

/// Compute the Cartesian product of all axes and build substitution maps.
fn cartesian_product(def: &MatrixDef) -> Result<Vec<SubstMap>> {
    let mut combos: Vec<SubstMap> = vec![HashMap::new()];

    for (axis_name, values) in &def.axes {
        let mut next = Vec::new();
        for combo in &combos {
            for val in values {
                let mut m = combo.clone();
                // The axis value itself.
                m.insert(axis_name.clone(), val.clone());
                // Flatten params for this axis+variant into the map.
                if let Some(variants) = def.params.get(axis_name) {
                    if let Some(fields) = variants.get(val) {
                        for (k, v) in fields {
                            m.insert(k.clone(), v.clone());
                        }
                    }
                }
                next.push(m);
            }
        }
        combos = next;
    }

    Ok(combos)
}

/// Recursively substitute `${matrix.<key>}` placeholders in a `toml::Value` tree.
fn substitute_value(val: &mut Value, map: &SubstMap) {
    match val {
        Value::String(s) => {
            *s = substitute_string(s, map);
        }
        Value::Array(arr) => {
            for item in arr {
                substitute_value(item, map);
            }
        }
        Value::Table(tbl) => {
            let keys: Vec<String> = tbl.keys().cloned().collect();
            for k in keys {
                if let Some(v) = tbl.get_mut(&k) {
                    substitute_value(v, map);
                }
            }
        }
        // Integers, floats, bools, datetimes — leave as-is.
        _ => {}
    }
}

/// Replace all `${matrix.<key>}` occurrences in a string.
fn substitute_string(s: &str, map: &SubstMap) -> String {
    let mut result = s.to_string();
    for (key, val) in map {
        let placeholder = format!("${{matrix.{key}}}");
        result = result.replace(&placeholder, val);
    }
    result
}

/// Expand a parsed TOML sim file into one or more TOML value trees.
///
/// If the file has no `[matrix]` table, returns the original tree as a single element.
/// Otherwise, returns one tree per matrix combination with all `${matrix.*}` placeholders
/// substituted.
pub fn expand_matrix(mut root: toml::value::Table) -> Result<Vec<toml::value::Table>> {
    let Some(def) = extract_matrix(&mut root).context("parse [matrix]")? else {
        return Ok(vec![root]);
    };

    let combos = cartesian_product(&def)?;
    let mut results = Vec::with_capacity(combos.len());

    for map in &combos {
        let mut tree = Value::Table(root.clone());
        substitute_value(&mut tree, map);
        match tree {
            Value::Table(t) => results.push(t),
            _ => unreachable!(),
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> toml::value::Table {
        toml::from_str::<toml::value::Table>(s).unwrap()
    }

    #[test]
    fn no_matrix_passthrough() {
        let root = parse(
            r#"
            [sim]
            name = "hello"
            "#,
        );
        let expanded = expand_matrix(root.clone()).unwrap();
        assert_eq!(expanded.len(), 1);
        // Matrix key removed (there was none), rest unchanged.
        assert_eq!(expanded[0], root);
    }

    #[test]
    fn single_axis() {
        let root = parse(
            r#"
            [matrix]
            topo = ["1to1", "1to3"]

            [sim]
            name = "test-${matrix.topo}"
            topology = "${matrix.topo}-public"
            "#,
        );
        let expanded = expand_matrix(root).unwrap();
        assert_eq!(expanded.len(), 2);

        let name0 = expanded[0]["sim"]["name"].as_str().unwrap();
        let name1 = expanded[1]["sim"]["name"].as_str().unwrap();
        assert_eq!(name0, "test-1to1");
        assert_eq!(name1, "test-1to3");

        let topo0 = expanded[0]["sim"]["topology"].as_str().unwrap();
        assert_eq!(topo0, "1to1-public");
    }

    #[test]
    fn multi_axis_cartesian() {
        let root = parse(
            r#"
            [matrix]
            topo = ["1to1", "1to3"]
            size = ["1G", "10G"]

            [sim]
            name = "test-${matrix.topo}-${matrix.size}"
            "#,
        );
        let expanded = expand_matrix(root).unwrap();
        assert_eq!(expanded.len(), 4);

        let names: Vec<&str> = expanded
            .iter()
            .map(|t| t["sim"]["name"].as_str().unwrap())
            .collect();
        // TOML tables are unordered (BTreeMap), so axis iteration is alphabetical.
        // "size" < "topo", so size varies first.
        assert_eq!(
            names,
            &[
                "test-1to1-1G",
                "test-1to3-1G",
                "test-1to1-10G",
                "test-1to3-10G"
            ]
        );
    }

    #[test]
    fn params_flattened() {
        let root = parse(
            r#"
            [matrix]
            cond = ["clean", "impaired"]

            [matrix.params.cond]
            clean = { latency = "0", rate = "0", impaired = "false" }
            impaired = { latency = "200", rate = "4000", impaired = "true" }

            [sim]
            name = "test-${matrix.cond}"

            [[step]]
            action = "set-link-condition"
            device = "fetcher"
            when = "${matrix.impaired}"

            [step.condition]
            latency_ms = "${matrix.latency}"
            rate_kbit = "${matrix.rate}"
            "#,
        );
        let expanded = expand_matrix(root).unwrap();
        assert_eq!(expanded.len(), 2);

        // clean variant
        let step0 = &expanded[0]["step"].as_array().unwrap()[0];
        assert_eq!(step0["when"].as_str().unwrap(), "false");
        assert_eq!(step0["condition"]["latency_ms"].as_str().unwrap(), "0");

        // impaired variant
        let step1 = &expanded[1]["step"].as_array().unwrap()[0];
        assert_eq!(step1["when"].as_str().unwrap(), "true");
        assert_eq!(step1["condition"]["latency_ms"].as_str().unwrap(), "200");
    }

    #[test]
    fn substitution_in_arrays() {
        let root = parse(
            r#"
            [matrix]
            size = ["1G", "10G"]

            [[step]]
            action = "run"
            device = "fetcher"
            cmd = ["echo", "--size=${matrix.size}"]
            "#,
        );
        let expanded = expand_matrix(root).unwrap();
        assert_eq!(expanded.len(), 2);

        let cmd0 = expanded[0]["step"].as_array().unwrap()[0]["cmd"]
            .as_array()
            .unwrap();
        assert_eq!(cmd0[1].as_str().unwrap(), "--size=1G");

        let cmd1 = expanded[1]["step"].as_array().unwrap()[0]["cmd"]
            .as_array()
            .unwrap();
        assert_eq!(cmd1[1].as_str().unwrap(), "--size=10G");
    }
}
