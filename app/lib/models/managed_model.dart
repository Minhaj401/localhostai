class ManagedModel {
  const ManagedModel({
    required this.id,
    required this.name,
    required this.bytes,
    required this.installed,
    this.description = '',
    this.layers = 0,
    this.parameters = 0,
    this.license = '',
    this.recommended = false,
    this.catalog = true,
    this.custom = false,
  });

  final String id;
  final String name;
  final String description;
  final int bytes;
  final int layers;
  final double parameters;
  final String license;
  final bool installed;
  final bool recommended;
  final bool catalog;
  final bool custom;

  bool get canPreview => !custom || installed;

  String get quantization {
    final match = RegExp(
            r'(?:^|[._/\s-])((?:i?q\d)[a-z0-9_]*|bf16|f16|fp16)(?=[.\s/-]|$)',
            caseSensitive: false)
        .firstMatch('$name $id');
    return match?.group(1)?.toUpperCase() ?? 'GGUF';
  }

  String get sizeLabel => bytes > 0
      ? '${(bytes / (1024 * 1024 * 1024)).toStringAsFixed(2)} GB'
      : 'Local GGUF';

  factory ManagedModel.fromCatalog(Map<String, dynamic> json) => ManagedModel(
        id: json['id'] as String,
        name: json['display_name'] as String? ?? json['id'] as String,
        description: json['description'] as String? ?? '',
        bytes: (json['bytes'] as num?)?.toInt() ?? 0,
        layers: (json['layers'] as num?)?.toInt() ?? 0,
        parameters: (json['params_b'] as num?)?.toDouble() ?? 0,
        license: json['license'] as String? ?? '',
        installed: json['installed'] == true,
        recommended: json['recommended'] == true,
        custom: json['custom'] == true,
      );

  factory ManagedModel.fromRegistered(Map<String, dynamic> json) =>
      ManagedModel(
        id: json['name'] as String,
        name: json['name'] as String,
        bytes: ((json['size_mb'] as num?)?.toInt() ?? 0) * 1024 * 1024,
        layers: (json['layers'] as num?)?.toInt() ?? 0,
        installed: true,
        catalog: false,
      );
}
