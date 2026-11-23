# ShadowPainel para Android

Este diretório contém um cliente Android companion feito com Expo. Ele se conecta ao ShadowPainel que está rodando no computador e permite acompanhar o servidor, iniciar/parar a instância ativa, trocar o perfil, ver métricas e consultar o console recente.

## Preparar o computador

O celular e o computador precisam estar na mesma rede. Inicie o desktop com o backend escutando na rede local:

```bash
SHADOWPAINEL_ADDR=0.0.0.0:4173 SHADOWPAINEL_PUBLIC_ADDR=127.0.0.1:4173 shadowpainel-desktop
```

Se estiver usando o pacote instalado pelo AUR ou o binário do release, use o mesmo ambiente na frente do comando que inicia `shadowpainel-desktop`. Descubra o IP local do computador com `ip -4 addr` e libere a porta `4173` no firewall local, se necessário.

No aplicativo, informe, por exemplo, `http://192.168.1.25:4173`. Não use `localhost` ou `127.0.0.1` no celular, pois esses endereços apontam para o próprio Android.

O modo remoto deve ser usado apenas em uma rede confiável. A API existente foi projetada originalmente para uso local e ainda não substitui uma camada de autenticação para exposição na internet; não encaminhe essa porta diretamente para a internet.

## Desenvolvimento

Instale Node.js e o Expo CLI por meio das dependências do projeto:

```bash
cd mobile
npm install
npm start
```

Abra o QR code no Expo Go ou execute `npm run android` com um emulador Android preparado. Para uma build instalável, use o fluxo de build do Expo/EAS ou gere o projeto nativo com `npx expo prebuild` em uma máquina com Android SDK.

## Escopo atual

A primeira versão prioriza o controle remoto seguro para a rede local: conexão ao painel, status, métricas, iniciar/parar, seleção de servidores e console recente. Recursos avançados continuam disponíveis na interface desktop/web do ShadowPainel.
