use serde_json::Number;

use std::collections::HashSet;

use crate::types::Warning;

pub(crate) fn clamped_number_from_f32(
    parameter: &str,
    value: f32,
    min: f32,
    max: f32,
    warnings: &mut Vec<Warning>,
) -> Option<Number> {
    if !value.is_finite() {
        warnings.push(Warning::Compatibility {
            feature: parameter.to_string(),
            details: format!("{parameter} must be a finite number; dropping invalid value"),
        });
        return None;
    }

    let mut clamped = value;
    if value > max {
        warnings.push(Warning::Clamped {
            parameter: parameter.to_string(),
            original: value,
            clamped_to: max,
        });
        clamped = max;
    } else if value < min {
        warnings.push(Warning::Clamped {
            parameter: parameter.to_string(),
            original: value,
            clamped_to: min,
        });
        clamped = min;
    }

    Number::from_f64(clamped as f64)
}

pub(crate) fn sanitize_stop_sequences(
    sequences: &[String],
    max: Option<usize>,
    warnings: &mut Vec<Warning>,
) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();

    for seq in sequences {
        let trimmed = seq.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    if let Some(max) = max {
        if out.len() > max {
            warnings.push(Warning::Compatibility {
                feature: "stop_sequences".to_string(),
                details: format!(
                    "provider supports at most {max} stop sequences; truncating from {} to {max}",
                    out.len()
                ),
            });
            out.truncate(max);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_emits_warning_and_clamps() {
        let mut warnings = Vec::new();
        let n =
            clamped_number_from_f32("temperature", 2.5, 0.0, 2.0, &mut warnings).expect("number");
        assert_eq!(n.as_f64(), Some(2.0));
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Clamped { parameter, original, clamped_to } if parameter == "temperature" && *original == 2.5 && *clamped_to == 2.0
        )));
    }

    #[test]
    fn non_finite_value_is_dropped_with_warning() {
        let mut warnings = Vec::new();
        let n = clamped_number_from_f32("temperature", f32::NAN, 0.0, 2.0, &mut warnings);
        assert!(n.is_none());
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "temperature"
        )));
    }

    #[test]
    fn stop_sequences_sanitizes_and_dedups() {
        let mut warnings = Vec::new();
        let out = sanitize_stop_sequences(
            &[
                " a ".to_string(),
                "a".to_string(),
                "".to_string(),
                " ".to_string(),
                "b".to_string(),
            ],
            None,
            &mut warnings,
        );
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn stop_sequences_truncates_with_warning() {
        let mut warnings = Vec::new();
        let out = sanitize_stop_sequences(
            &[
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
                "e".to_string(),
            ],
            Some(4),
            &mut warnings,
        );
        assert_eq!(
            out,
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string()
            ]
        );
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, .. } if feature == "stop_sequences"
        )));
    }
}
