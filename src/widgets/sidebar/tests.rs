use super::*;

#[test]
fn saturation_preserves_alpha_and_full_color() {
    assert_eq!(
        saturation_matrix(1.0),
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0
        ]
    );

    let gray = saturation_matrix(0.0);
    for channel in 0..3 {
        assert_eq!(gray[channel], gray[0]);
        assert_eq!(gray[4 + channel], gray[4]);
        assert_eq!(gray[8 + channel], gray[8]);
    }

    assert_eq!(gray[15], 1.0);
    assert_eq!(saturation_matrix(-1.0), gray);
    assert_eq!(saturation_matrix(2.0), saturation_matrix(1.0));
}
