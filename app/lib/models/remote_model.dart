/// Model discovery metadata comes from the coordinator. Downloads and tokens
/// stay on that machine; the phone sends only the chosen source back to it.
class RemoteModel {
  const RemoteModel({
    required this.id,
    required this.name,
    required this.bytes,
    required this.source,
    this.description = '',
    this.license = '',
    this.fileCount = 1,
  });

  final String id;
  final String name;
  final int bytes;
  final String description;
  final String license;
  final int fileCount;
  final Map<String, dynamic> source;

  String get sizeLabel =>
      '${(bytes / (1024 * 1024 * 1024)).toStringAsFixed(2)} GiB';

  factory RemoteModel.fromJson(Map<String, dynamic> value) => RemoteModel(
        id: value['id'] as String,
        name: value['display_name'] as String,
        bytes: (value['bytes'] as num).toInt(),
        description: value['description'] as String? ?? '',
        license: value['license'] as String? ?? '',
        fileCount: (value['files'] as List?)?.length ?? 1,
        source: Map<String, dynamic>.from(value['source'] as Map),
      );
}

class HubResults {
  const HubResults(
      {this.repositories = const [], this.models = const [], this.repository});

  final List<String> repositories;
  final List<RemoteModel> models;
  final String? repository;

  factory HubResults.fromJson(Map<String, dynamic> value) => HubResults(
        repositories: (value['repositories'] as List? ?? const [])
            .whereType<Map>()
            .map((item) => item['id'] as String)
            .toList(),
        models: (value['models'] as List? ?? const [])
            .whereType<Map>()
            .map(
                (item) => RemoteModel.fromJson(Map<String, dynamic>.from(item)))
            .toList(),
        repository: value['repository'] as String?,
      );
}
