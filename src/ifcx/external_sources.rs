//! Pure CAD path / native URI-reference conversion. Resolution belongs to hosts.
use crate::Error;
use ifccad::IfccadExternalSourceLocation as Location;

fn error(message: &str) -> Error {
    format!("external source location: {message}").into()
}
fn encode(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            use std::fmt::Write;
            write!(&mut out, "%{b:02X}").unwrap();
        }
    }
    out
}
fn decode(uri: &str) -> Result<String, Error> {
    let bytes = uri.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = uri
                .get(i + 1..i + 3)
                .ok_or_else(|| error("invalid URI escape"))?;
            out.push(u8::from_str_radix(pair, 16).map_err(|_| error("invalid URI escape"))?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(out).map_err(|_| error("file URI must encode UTF-8"))?;
    if path.contains('\0') {
        return Err(error("file path contains NUL"));
    }
    Ok(path)
}

/// Convert a literal CAD path, escaping filename bytes exactly once.
pub fn external_source_location_from_cad_path(path: &str) -> Result<Location, Error> {
    if path.is_empty() || path.contains('\0') {
        return Err(error("empty or invalid CAD path"));
    }
    let path = path.replace('\\', "/");
    let bytes = path.as_bytes();
    let location = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes.get(2) != Some(&b'/') {
            return Err(error("drive-relative paths have no portable basis"));
        }
        Location::UriReference {
            uri: format!("file:///{}:{}", &path[..1], encode(&path[2..])),
        }
    } else if path
        .split('/')
        .next()
        .is_some_and(|first| first.contains(':'))
    {
        Location::UriReference { uri: path }
    } else if let Some(rest) = path.strip_prefix("//") {
        let (host, tail) = rest
            .split_once('/')
            .ok_or_else(|| error("UNC path requires a share"))?;
        if host.is_empty() || tail.is_empty() {
            return Err(error("UNC path requires a server and share"));
        }
        Location::UriReference {
            uri: format!("file://{}/{}", encode(host), encode(tail)),
        }
    } else if path.starts_with('/') {
        Location::UriReference {
            uri: format!("file://{}", encode(&path)),
        }
    } else if path.contains('/') {
        Location::UriReference { uri: encode(&path) }
    } else {
        Location::SearchFileName { file_name: path }
    };
    location.validate()?;
    Ok(location)
}

/// Map a directly representable native source to CAD path text.
/// Unsupported URI schemes, queries and fragments produce an explicit error.
pub fn external_source_location_to_cad_path(location: &Location) -> Result<String, Error> {
    location.validate()?;
    let Location::UriReference { uri } = location else {
        let Location::SearchFileName { file_name } = location else {
            unreachable!()
        };
        return Ok(file_name.clone());
    };
    if uri.contains(['?', '#']) {
        return Err(error(
            "URI query or fragment cannot identify a whole CAD file",
        ));
    }
    let scheme = uri
        .split('/')
        .next()
        .and_then(|first| first.split_once(':').map(|(s, _)| s));
    let path = if let Some(scheme) = scheme {
        if !scheme.eq_ignore_ascii_case("file") {
            return Err(error("URI scheme has no qualified DWG/DXF path mapping"));
        }
        let rest = &uri[scheme.len() + 1..];
        let path = if let Some(rest) = rest.strip_prefix("//") {
            let (host, path) = rest
                .split_once('/')
                .ok_or_else(|| error("file URI requires an absolute path"))?;
            if host.contains(['@', ':']) {
                return Err(error("file URI authority is not a file server"));
            }
            if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
                format!("/{path}")
            } else {
                format!("//{host}/{path}")
            }
        } else {
            if !rest.starts_with('/') {
                return Err(error("file URI requires an absolute path"));
            }
            rest.into()
        };
        let path = decode(&path)?;
        if path.as_bytes().get(2) == Some(&b':') && path.starts_with('/') {
            path[1..].to_owned()
        } else {
            path
        }
    } else {
        let path = decode(uri)?;
        // Decoded colons/backslashes must not create a drive or UNC path.
        if path.contains('\\') || path.split('/').next().is_some_and(|s| s.contains(':')) {
            return Err(error(
                "relative URI cannot map to a drive or backslash path",
            ));
        }
        if path.starts_with('/') {
            return Err(error("absolute native paths require file URI"));
        }
        if path.contains('/') {
            path
        } else {
            format!("./{path}")
        }
    };
    if path.is_empty() {
        return Err(error("file URI has no filename"));
    }
    Ok(path)
}

/// Check that every CAD XREF locator has a qualified whole-file mapping.
/// Native editing can retain unsupported URIs as unresolved references.
pub fn validate_cad_external_sources(document: &crate::Document) -> Result<(), Error> {
    for record in document
        .block_records
        .iter()
        .filter(|r| r.flags.is_xref || r.flags.is_xref_overlay)
    {
        let location = external_source_location_from_cad_path(&record.xref_path)?;
        external_source_location_to_cad_path(&location)
            .map_err(|e| error(&format!("XREF {}: {e}", record.name)))?;
    }
    Ok(())
}

fn cad_source_metadata(document: &crate::Document) -> ([f64; 3], Option<String>, Option<String>) {
    let code = document.header.insertion_units;
    (
        super::array(document.header.model_space_insertion_base),
        usize::try_from(code)
            .ok()
            .and_then(|i| super::UNITS.get(i))
            .map(|v| (*v).to_owned()),
        ifccad::normalize_ifccad_fingerprint_guid(&document.header.fingerprint_guid),
    )
}

/// Metadata of an actually opened DWG. This never invents a source UUID.
pub fn dwg_external_source_content(
    document: &crate::Document,
) -> ifccad::IfccadExternalSourceContent {
    let (base_point, insertion_unit, fingerprint_guid) = cad_source_metadata(document);
    ifccad::IfccadExternalSourceContent::Dwg {
        base_point,
        insertion_unit,
        fingerprint_guid,
    }
}

/// Metadata of an actually opened DXF, including its available header identity.
pub fn dxf_external_source_content(
    document: &crate::Document,
) -> ifccad::IfccadExternalSourceContent {
    let (base_point, insertion_unit, fingerprint_guid) = cad_source_metadata(document);
    ifccad::IfccadExternalSourceContent::Dxf {
        base_point,
        insertion_unit,
        fingerprint_guid,
    }
}

/// Metadata for a native source projected into a host drawing temporarily.
/// Its own external-source graph is outside the nonrecursive source slice.
pub fn ifccad_external_source_content(
    drawing: &ifccad::IfccadDocument,
) -> Result<ifccad::IfccadExternalSourceContent, Error> {
    ifccad::validate_ifccad_document(drawing)?;
    if !drawing.external_sources.is_empty() {
        return Err("This IFCCAD source contains its own external references; nested XREF sources are not supported yet".into());
    }
    Ok(ifccad::IfccadExternalSourceContent::Ifccad {
        base_point: [0.; 3],
        insertion_unit: Some(drawing.length_unit.clone()),
    })
}

/// Expected XREF identity from the physical feature/filename key, independently
/// of the main drawing header and of the source's last-loaded session identity.
pub fn dwg_xref_fingerprint_guid(
    document: &crate::Document,
    record: &acadrust::tables::BlockRecord,
) -> Result<Option<String>, Error> {
    let mut candidates = document.file_dependencies.iter().filter(|d| {
        d.feature.eq_ignore_ascii_case("Acad:XRef")
            && d.file_name
                .replace('\\', "/")
                .eq_ignore_ascii_case(&record.xref_path.replace('\\', "/"))
    });
    let dependency = candidates.next();
    if candidates.next().is_some() {
        return Err(format!("ambiguous FileDepList identity for XREF {}", record.name).into());
    }
    Ok(dependency
        .and_then(|d| d.fingerprint_guid.as_deref())
        .and_then(ifccad::normalize_ifccad_fingerprint_guid))
}
