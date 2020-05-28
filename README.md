# Sourcelynk

Sourcelynk is a command line tool to generate and embed information about
where to find the source files into an ELF.

Sourcelynk is a Rust implementation of Microsoft's [SourceLink]. Though
[SourceLink] is targetted towards PDBs, Sourcelynk attempts to work for ELF,
PDB, and Mach-O files.

[SourceLink]: https://github.com/dotnet/designs/blob/master/accepted/2020/diagnostics/source-link.md#source-link-json-schema

## How does it work?

Sourcelynk will generate the sourcelink [json] for any binary (with symbols)
that you point it at and then (optionally) embed that json into the binary
itself. How this works is different per binary format.

[json]: https://github.com/dotnet/designs/blob/master/accepted/2020/diagnostics/source-link.md#source-link-json-schema

### ELFs

ELFs are (currently) the only tool supported by Sourcelynk. The sourcelink
JSON file created by Sourcelynk is stored in a new section of the ELF called
".debug_sourcelink".

Currently no debuggers support using source link JSON in ELF files.