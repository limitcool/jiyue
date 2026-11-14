import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'bridge/vault_service.dart';
import 'theme/theme_provider.dart';
import 'views/vault_page.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  
  // 初始化笔记库服务
  final vaultService = VaultService.instance;
  await vaultService.init();

  // 初始化主题提供者
  final themeProvider = ThemeProvider();

  runApp(JiyueMobileApp(
    themeProvider: themeProvider,
    vaultService: vaultService,
  ));
}

class JiyueMobileApp extends StatefulWidget {
  final ThemeProvider themeProvider;
  final VaultService vaultService;

  const JiyueMobileApp({
    super.key,
    required this.themeProvider,
    required this.vaultService,
  });

  @override
  State<JiyueMobileApp> createState() => _JiyueMobileAppState();
}

class _JiyueMobileAppState extends State<JiyueMobileApp> {
  @override
  void initState() {
    super.initState();
    widget.themeProvider.addListener(_onThemeChanged);
  }

  @override
  void dispose() {
    widget.themeProvider.removeListener(_onThemeChanged);
    super.dispose();
  }

  void _onThemeChanged() {
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final colors = widget.themeProvider.colors;

    // 同步设置系统状态栏颜色
    SystemChrome.setSystemUIOverlayStyle(
      SystemUiOverlayStyle(
        statusBarColor: Colors.transparent,
        statusBarIconBrightness: colors.isDark ? Brightness.light : Brightness.dark,
        systemNavigationBarColor: colors.chrome,
        systemNavigationBarIconBrightness: colors.isDark ? Brightness.light : Brightness.dark,
      ),
    );

    return MaterialApp(
      title: '霁月 Jiyue',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: colors.isDark ? Brightness.dark : Brightness.light,
        scaffoldBackgroundColor: colors.chrome,
        fontFamily: 'Noto Sans SC',
        colorScheme: ColorScheme(
          brightness: colors.isDark ? Brightness.dark : Brightness.light,
          primary: colors.accent,
          onPrimary: colors.onAccent,
          secondary: colors.accent,
          onSecondary: colors.onAccent,
          surface: colors.sheet,
          onSurface: colors.textPrimary,
          error: Colors.redAccent,
          onError: Colors.white,
        ),
      ),
      home: VaultPage(
        themeProvider: widget.themeProvider,
        vaultService: widget.vaultService,
      ),
    );
  }
}
