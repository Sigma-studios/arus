// arus as an app: the hosted arus web app in its own window, rendered by the
// platform's web engine. Logins, choreographies and music all stay on the
// server; the app keeps its own cookies, so you stay logged in.
//
// - Windows: Edge WebView2 (windows_view.dart).
// - Android, iOS, macOS: the system webview (webkit_view.dart).
// - Linux: WebKitGTK, set up by the native runner (linux/runner); this Dart
//   code does not run there.
//
// Another server: `--url http://localhost:8080` or ARUS_URL (desktop).

import 'dart:io';

import 'package:flutter/material.dart';

import 'common.dart';
import 'webkit_view.dart';
import 'windows_view.dart';

void main(List<String> args) {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(ArusApp(url: startUrl(args)));
}

class ArusApp extends StatelessWidget {
  const ArusApp({super.key, required this.url});

  final String url;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'arus',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        scaffoldBackgroundColor: background,
        colorScheme: const ColorScheme.dark(primary: accent, surface: background),
      ),
      home: Platform.isWindows ? WindowsView(url: url) : WebKitView(url: url),
    );
  }
}
