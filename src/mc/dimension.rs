use nbt::CompoundTag;

/// Create lobby dimension from the given codec.
///
/// This creates a dimension suitable for the lobby that should be suitable for the current server
/// version.
pub fn lobby_dimension(codec: &CompoundTag) -> CompoundTag {
    // Retrieve dimension types from codec
    let dimension_types = match codec.get_compound_tag("minecraft:dimension_type") {
        Ok(types) => types,
        Err(_) => return lobby_default_dimension(),
    };

    // Get base dimension
    let mut base = lobby_base_dimension(dimension_types);

    // Change known properties on base to get more desirable dimension
    base.insert_i8("piglin_safe", 1);
    base.insert_f32("ambient_light", 0.0);
    // base.insert_str("infiniburn", "minecraft:infiniburn_end");
    base.insert_i8("respawn_anchor_works", 0);
    base.insert_i8("has_skylight", 0);
    base.insert_i8("bed_works", 0);
    base.insert_str("effects", "minecraft:the_end");
    base.insert_i64("fixed_time", 0);
    base.insert_i8("has_raids", 0);
    base.insert_i32("min_y", 0);
    base.insert_i32("height", 16);
    base.insert_i32("logical_height", 16);
    base.insert_f64("coordinate_scale", 1.0);
    base.insert_i8("ultrawarm", 0);
    base.insert_i8("has_ceiling", 0);

    base
}

/// Get lobby base dimension.
///
/// This retrieves the most desirable dimension to use as base for the lobby from the given list of
/// `dimension_types`.
///
/// If no dimension is found in the given tag, a default one will be returned.
fn lobby_base_dimension(dimension_types: &CompoundTag) -> CompoundTag {
    // The dimension types we prefer the most, in order
    let preferred = vec![
        "minecraft:the_end",
        "minecraft:the_nether",
        "minecraft:the_overworld",
    ];

    let dimensions = dimension_types.get_compound_tag_vec("value").unwrap();

    for name in preferred {
        if let Some(dimension) = dimensions
            .iter()
            .find(|d| d.get_str("name").map(|n| n == name).unwrap_or(false))
        {
            if let Ok(dimension) = dimension.get_compound_tag("element") {
                return dimension.clone();
            }
        }
    }

    // Return first dimension
    if let Some(dimension) = dimensions.first() {
        if let Ok(dimension) = dimension.get_compound_tag("element") {
            return dimension.clone();
        }
    }

    // Fall back to default dimension
    lobby_default_dimension()
}

/// Default lobby dimension codec from resource file.
///
/// This likely breaks if the Minecraft version doesn't match exactly.
/// Please use an up-to-date coded from the server instead.
pub fn default_dimension_codec() -> CompoundTag {
    snbt_to_compound_tag(include_str!("../../res/dimension_codec.snbt"))
}

/// Default lobby dimension from resource file.
///
/// This likely breaks if the Minecraft version doesn't match exactly.
/// Please use `lobby_dimension` with an up-to-date coded from the server instead.
fn lobby_default_dimension() -> CompoundTag {
    snbt_to_compound_tag(include_str!("../../res/dimension.snbt"))
}

/// Read NBT CompoundTag from SNBT.
fn snbt_to_compound_tag(data: &str) -> CompoundTag {
    use quartz_nbt::io::{write_nbt, Flavor};
    use quartz_nbt::snbt;

    // Parse SNBT data
    let compound = snbt::parse(data).expect("failed to parse SNBT");

    // Encode to binary
    let mut binary = Vec::new();
    write_nbt(&mut binary, None, &compound, Flavor::Uncompressed)
        .expect("failed to encode NBT CompoundTag as binary");

    // Parse binary with usable NBT create
    bin_to_compound_tag(&binary)
}

/// Read NBT CompoundTag from SNBT.
fn bin_to_compound_tag(data: &[u8]) -> CompoundTag {
    use nbt::decode::read_compound_tag;
    read_compound_tag(&mut &*data).unwrap()
}
