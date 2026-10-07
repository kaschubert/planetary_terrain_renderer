//! The stations file read and asked for names: a station by its own id, a platform by its
//! station's, and the words every name carries dropped.

use super::*;

const SAMPLE: &str = "\
# Auckland's train stations, a cut of the file
stop_id,parent,code,latitude,longitude,name
105-474861ff,,105,-36.89714,174.69910,Avondale Train Station
9000-abc,,9000,-36.84400,174.76800,Waitemata Train Station
9001-c9315521,9000-abc,9001,-36.84410,174.76810,Waitemata Train Station 1
9002-ffdd37b7,9000-abc,9002,-36.84420,174.76820,Waitemata Train Station 2
1741-4e586aef,,1741,-36.94000,174.84000,Titi Street, Otahuhu Train Station
";

#[test]
fn the_file_reads_with_stations_and_their_platforms_and_a_comma_in_a_name() {
    let stations = Stations::parse(SAMPLE).unwrap();
    assert_eq!(stations.stops.len(), 5);
    assert_eq!(stations.stations().count(), 3);

    let avondale = stations.get("105-474861ff").unwrap();
    assert_eq!(avondale.parent, None);
    assert_eq!(avondale.code, "105");
    assert_eq!(avondale.latitude, -36.89714);
    assert_eq!(avondale.longitude, 174.69910);
    assert!(avondale.is_station());

    let platform = stations.get("9002-ffdd37b7").unwrap();
    assert_eq!(platform.parent.as_deref(), Some("9000-abc"));
    assert!(!platform.is_station());

    assert_eq!(
        stations.get("1741-4e586aef").unwrap().name,
        "Titi Street, Otahuhu Train Station"
    );
}

#[test]
fn a_stop_id_names_its_station_through_its_parent() {
    let stations = Stations::parse(SAMPLE).unwrap();
    assert_eq!(stations.name_of("105-474861ff"), Some("Avondale"));
    assert_eq!(stations.name_of("9000-abc"), Some("Waitemata"));
    assert_eq!(stations.name_of("9001-c9315521"), Some("Waitemata"));
    assert_eq!(stations.name_of("9002-ffdd37b7"), Some("Waitemata"));
    assert_eq!(
        stations.name_of("1741-4e586aef"),
        Some("Titi Street, Otahuhu")
    );
    assert_eq!(stations.name_of("nowhere"), None);
}

#[test]
fn the_short_name_drops_the_words_and_the_platform_number() {
    assert_eq!(short_name("Avondale Train Station"), "Avondale");
    assert_eq!(short_name("Waitemata Train Station 2"), "Waitemata");
    assert_eq!(short_name("Baldwin Ave Train Station"), "Baldwin Ave");
    assert_eq!(short_name("Britomart"), "Britomart");
    assert_eq!(short_name(""), "");
}

#[test]
fn a_bad_header_or_row_is_refused_by_name() {
    let error = Stations::parse("stop_id,name\n1,,1,0,0,x\n").unwrap_err();
    assert!(error.contains("header"), "{error}");

    let error = Stations::parse(&format!("{HEADER}\n1,,1,0,0\n")).unwrap_err();
    assert!(
        error.contains("row 1") && error.contains("5 fields"),
        "{error}"
    );

    let error = Stations::parse(&format!("{HEADER}\n1,,1,south,0,x\n")).unwrap_err();
    assert!(
        error.contains("row 1") && error.contains("latitude"),
        "{error}"
    );

    assert!(Stations::parse("").unwrap_err().contains("no header"));
    assert_eq!(Stations::parse(HEADER).unwrap().stops.len(), 0);
}
